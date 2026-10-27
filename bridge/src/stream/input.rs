//! Browser → host input over WebRTC data channels.
//!
//! Wire format: one type byte, then a fixed little-endian payload (see `web/src/lib/stream/input.ts`).
//! Mouse motion travels on an unordered, no-retransmit channel ("input"); everything with
//! state (buttons, keys, controllers) on an ordered, reliable one ("input-reliable").

use std::ffi::c_char;

use moonlight_common_sys as ml;

use super::connection::Connection;

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    MouseMove { dx: i16, dy: i16 },
    MousePosition { x: i16, y: i16, ref_width: i16, ref_height: i16 },
    MouseButton { pressed: bool, button: u8 },
    Scroll { amount: i16 },
    HScroll { amount: i16 },
    /// `vk` is a Windows virtual-key code (US layout).
    Key { pressed: bool, vk: u16, modifiers: u8 },
    Controller(ControllerState),
    ControllerArrival { index: u8, active_mask: u16, kind: u8, supported_buttons: u32, capabilities: u16 },
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControllerState {
    pub index: u8,
    pub active_mask: u16,
    pub buttons: u32,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub left_stick: (i16, i16),
    pub right_stick: (i16, i16),
}

mod tag {
    pub const MOUSE_MOVE: u8 = 0x01;
    pub const MOUSE_POSITION: u8 = 0x02;
    pub const MOUSE_BUTTON: u8 = 0x03;
    pub const SCROLL: u8 = 0x04;
    pub const HSCROLL: u8 = 0x05;
    pub const KEY: u8 = 0x06;
    pub const CONTROLLER: u8 = 0x07;
    pub const CONTROLLER_ARRIVAL: u8 = 0x08;
    pub const TEXT: u8 = 0x09;
}

/// Little-endian cursor over a message payload.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_le_bytes)
    }
    fn i16(&mut self) -> Option<i16> {
        self.take().map(i16::from_le_bytes)
    }
    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }
}

pub fn decode(msg: &[u8]) -> Option<InputEvent> {
    let (&kind, payload) = msg.split_first()?;
    let mut r = Reader(payload);
    Some(match kind {
        tag::MOUSE_MOVE => InputEvent::MouseMove { dx: r.i16()?, dy: r.i16()? },
        tag::MOUSE_POSITION => {
            InputEvent::MousePosition { x: r.i16()?, y: r.i16()?, ref_width: r.i16()?, ref_height: r.i16()? }
        }
        tag::MOUSE_BUTTON => InputEvent::MouseButton { pressed: r.u8()? != 0, button: r.u8()? },
        tag::SCROLL => InputEvent::Scroll { amount: r.i16()? },
        tag::HSCROLL => InputEvent::HScroll { amount: r.i16()? },
        tag::KEY => InputEvent::Key { pressed: r.u8()? != 0, vk: r.u16()?, modifiers: r.u8()? },
        tag::CONTROLLER => InputEvent::Controller(ControllerState {
            index: r.u8()?,
            active_mask: r.u16()?,
            buttons: r.u32()?,
            left_trigger: r.u8()?,
            right_trigger: r.u8()?,
            left_stick: (r.i16()?, r.i16()?),
            right_stick: (r.i16()?, r.i16()?),
        }),
        tag::CONTROLLER_ARRIVAL => InputEvent::ControllerArrival {
            index: r.u8()?,
            active_mask: r.u16()?,
            kind: r.u8()?,
            supported_buttons: r.u32()?,
            capabilities: r.u16()?,
        },
        tag::TEXT => InputEvent::Text(String::from_utf8(payload.to_vec()).ok()?),
        _ => return None,
    })
}

/// Forward one event to the host. No-op when no connection is up.
pub fn apply(event: &InputEvent) {
    if !Connection::is_active() {
        return;
    }
    // SAFETY: the Li* input functions only queue packets and are safe to call from any
    // thread between LiStartConnection and LiStopConnection (checked above).
    let rc = unsafe {
        match event {
            InputEvent::MouseMove { dx, dy } => ml::LiSendMouseMoveEvent(*dx, *dy),
            InputEvent::MousePosition { x, y, ref_width, ref_height } => {
                ml::LiSendMousePositionEvent(*x, *y, *ref_width, *ref_height)
            }
            InputEvent::MouseButton { pressed, button } => {
                if !(1..=5).contains(button) {
                    return;
                }
                let action = if *pressed { ml::BUTTON_ACTION_PRESS } else { ml::BUTTON_ACTION_RELEASE };
                ml::LiSendMouseButtonEvent(action as c_char, *button as i32)
            }
            InputEvent::Scroll { amount } => ml::LiSendHighResScrollEvent(*amount),
            InputEvent::HScroll { amount } => ml::LiSendHighResHScrollEvent(*amount),
            InputEvent::Key { pressed, vk, modifiers } => {
                let action = if *pressed { ml::KEY_ACTION_DOWN } else { ml::KEY_ACTION_UP };
                // High bit marks the code as a VK code, as official Moonlight clients send it.
                let code = (0x8000 | (*vk & 0xFF)) as i16;
                ml::LiSendKeyboardEvent2(code, action as c_char, *modifiers as c_char, 0)
            }
            InputEvent::Controller(c) => ml::LiSendMultiControllerEvent(
                c.index as i16,
                c.active_mask as i16,
                c.buttons as i32,
                c.left_trigger,
                c.right_trigger,
                c.left_stick.0,
                c.left_stick.1,
                c.right_stick.0,
                c.right_stick.1,
            ),
            InputEvent::ControllerArrival { index, active_mask, kind, supported_buttons, capabilities } => {
                ml::LiSendControllerArrivalEvent(*index, *active_mask, *kind, *supported_buttons, *capabilities)
            }
            InputEvent::Text(text) => ml::LiSendUtf8TextEvent(text.as_ptr() as *const c_char, text.len() as u32),
        }
    };
    if rc != 0 {
        tracing::trace!(?event, rc, "input event rejected");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_mouse_and_keys() {
        assert_eq!(decode(&[0x01, 0xFE, 0xFF, 0x05, 0x00]), Some(InputEvent::MouseMove { dx: -2, dy: 5 }));
        assert_eq!(
            decode(&[0x02, 10, 0, 20, 0, 0x80, 0x07, 0x38, 0x04]),
            Some(InputEvent::MousePosition { x: 10, y: 20, ref_width: 1920, ref_height: 1080 })
        );
        assert_eq!(decode(&[0x03, 1, 3]), Some(InputEvent::MouseButton { pressed: true, button: 3 }));
        assert_eq!(decode(&[0x04, 0x88, 0xFF]), Some(InputEvent::Scroll { amount: -120 }));
        assert_eq!(decode(&[0x06, 0, 0x41, 0x00, 0x02]), Some(InputEvent::Key { pressed: false, vk: 0x41, modifiers: 2 }));
        assert_eq!(decode("\x09héllo".as_bytes()), Some(InputEvent::Text("héllo".into())));
    }

    #[test]
    fn decodes_controller() {
        let mut msg = vec![0x07, 1, 0x03, 0x00];
        msg.extend(0x1000u32.to_le_bytes()); // A
        msg.extend([255, 0]);
        for v in [32767i16, -32768, 0, 100] {
            msg.extend(v.to_le_bytes());
        }
        assert_eq!(
            decode(&msg),
            Some(InputEvent::Controller(ControllerState {
                index: 1,
                active_mask: 3,
                buttons: 0x1000,
                left_trigger: 255,
                right_trigger: 0,
                left_stick: (32767, -32768),
                right_stick: (0, 100),
            }))
        );
    }

    #[test]
    fn rejects_truncated_and_unknown() {
        assert_eq!(decode(&[]), None);
        assert_eq!(decode(&[0x01, 0x00]), None, "truncated payload");
        assert_eq!(decode(&[0x07, 0, 0, 0]), None, "truncated controller");
        assert_eq!(decode(&[0xEE, 1, 2, 3]), None, "unknown type");
        assert_eq!(decode(&[0x09, 0xFF, 0xFE]), None, "invalid UTF-8");
    }
}
