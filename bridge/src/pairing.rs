//! GameStream PIN pairing.
//!
//! Flow (all over plain HTTP except the last step):
//! 1. `getservercert` — send salt + our cert; blocks until the user types the PIN into
//!    Sunshine. Both sides derive `key = H(salt || pin)[..16]`.
//! 2. `clientchallenge` — AES-ECB(key, random challenge). The host answers with
//!    `H(challenge || server_cert_sig || server_secret) || server_challenge`.
//! 3. `serverchallengeresp` — AES-ECB(key, H(server_challenge || client_cert_sig || client_secret)).
//!    The host reveals `server_secret || sign(server_secret)`; recomputing the hash from
//!    step 2 proves both sides used the same PIN.
//! 4. `clientpairingsecret` — `client_secret || sign(client_secret)`.
//! 5. `pairchallenge` over HTTPS with our client cert — confirms the host pinned it.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use openssl::{
    error::ErrorStack,
    hash::MessageDigest,
    memcmp,
    sign::{Signer, Verifier},
    symm::{Cipher, Crypter, Mode},
    x509::X509,
};
use serde::Serialize;

use crate::{
    identity::Identity,
    nvhttp::{child_text, parse_root, Endpoint, NvClient, NvError},
};

/// How long to wait for the user to enter the PIN on the host.
const PIN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
pub enum PairError {
    #[error("wrong PIN — check the PIN and try again")]
    WrongPin,
    #[error("another pairing is already in progress on the host")]
    AlreadyInProgress,
    #[error("the host rejected pairing{0}")]
    Rejected(&'static str),
    #[error("the host's response failed signature verification (possible MITM)")]
    BadSignature,
    #[error(transparent)]
    Nv(#[from] NvError),
    #[error("{step} failed: {source}")]
    Step { step: &'static str, source: NvError },
    #[error("crypto error: {0}")]
    Crypto(#[from] ErrorStack),
    #[error("malformed pairing response: {0}")]
    Malformed(&'static str),
}

#[derive(Clone, Copy)]
enum HashAlgo {
    Sha1,
    Sha256,
}

impl HashAlgo {
    /// GFE 7+ (and every Sunshine release) uses SHA-256; older GFE used SHA-1.
    fn for_app_version(major: u32) -> Self {
        if major >= 7 {
            Self::Sha256
        } else {
            Self::Sha1
        }
    }

    fn digest(self, parts: &[&[u8]]) -> Vec<u8> {
        let data = parts.concat();
        match self {
            Self::Sha1 => openssl::sha::sha1(&data).to_vec(),
            Self::Sha256 => openssl::sha::sha256(&data).to_vec(),
        }
    }

    fn len(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Sha256 => 32,
        }
    }
}

fn random<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    openssl::rand::rand_bytes(&mut buf).expect("rng");
    buf
}

/// Random 4-digit PIN.
pub fn generate_pin() -> String {
    format!("{:04}", u32::from_le_bytes(random::<4>()) % 10_000)
}

/// AES-128-ECB without padding; input is zero-padded to the block size.
fn aes_ecb(key: &[u8; 16], data: &[u8], mode: Mode) -> Result<Vec<u8>, ErrorStack> {
    let mut input = data.to_vec();
    input.resize(data.len().div_ceil(16) * 16, 0);
    let cipher = Cipher::aes_128_ecb();
    let mut c = Crypter::new(cipher, mode, key, None)?;
    c.pad(false);
    let mut out = vec![0; input.len() + cipher.block_size()];
    let n = c.update(&input, &mut out)?;
    let m = c.finalize(&mut out[n..])?;
    out.truncate(n + m);
    Ok(out)
}

/// The I/O-free crypto half of pairing; [`pair`] feeds it the host's responses.
struct Handshake<'a> {
    hash: HashAlgo,
    key: [u8; 16],
    identity: &'a Identity,
    client_challenge: [u8; 16],
    client_secret: [u8; 16],
    /// `H(client_challenge || server_cert_sig || server_secret)` as claimed by the host.
    server_response: Vec<u8>,
}

impl<'a> Handshake<'a> {
    fn new(hash: HashAlgo, salt: &[u8; 16], pin: &str, identity: &'a Identity) -> Self {
        let mut key = [0u8; 16];
        key.copy_from_slice(&hash.digest(&[salt, pin.as_bytes()])[..16]);
        Self {
            hash,
            key,
            identity,
            client_challenge: random(),
            client_secret: random(),
            server_response: Vec::new(),
        }
    }

    /// Step 2 request: encrypted client challenge.
    fn client_challenge(&self) -> Result<Vec<u8>, PairError> {
        Ok(aes_ecb(&self.key, &self.client_challenge, Mode::Encrypt)?)
    }

    /// Step 2 response → step 3 request.
    fn answer_server_challenge(&mut self, challenge_response: &[u8]) -> Result<Vec<u8>, PairError> {
        let plain = aes_ecb(&self.key, challenge_response, Mode::Decrypt)?;
        let hlen = self.hash.len();
        if plain.len() < hlen + 16 {
            return Err(PairError::Malformed("challengeresponse too short"));
        }
        self.server_response = plain[..hlen].to_vec();
        let server_challenge = &plain[hlen..hlen + 16];

        let client_sig = self.identity.cert.signature().as_slice();
        let answer = self.hash.digest(&[server_challenge, client_sig, &self.client_secret]);
        Ok(aes_ecb(&self.key, &answer, Mode::Encrypt)?)
    }

    /// Step 3 response → step 4 request. This is where a wrong PIN is detected.
    fn verify_server_secret(&self, server_cert: &X509, pairing_secret: &[u8]) -> Result<Vec<u8>, PairError> {
        if pairing_secret.len() <= 16 {
            return Err(PairError::Malformed("pairingsecret too short"));
        }
        let (server_secret, server_sig) = pairing_secret.split_at(16);

        let server_key = server_cert.public_key()?;
        let mut verifier = Verifier::new(MessageDigest::sha256(), &server_key)?;
        verifier.update(server_secret)?;
        if !verifier.verify(server_sig)? {
            return Err(PairError::BadSignature);
        }

        let expected = self.hash.digest(&[&self.client_challenge, server_cert.signature().as_slice(), server_secret]);
        if expected.len() != self.server_response.len() || !memcmp::eq(&expected, &self.server_response) {
            return Err(PairError::WrongPin);
        }

        let mut signer = Signer::new(MessageDigest::sha256(), &self.identity.key)?;
        signer.update(&self.client_secret)?;
        Ok([self.client_secret.as_slice(), &signer.sign_to_vec()?].concat())
    }
}

/// Tag transport errors with the pairing step they happened in.
fn at(step: &'static str) -> impl FnOnce(NvError) -> PairError {
    move |source| {
        tracing::debug!(step, "pairing request failed: {source}");
        PairError::Step { step, source }
    }
}

/// Check `<paired>1</paired>` and return the hex-decoded value of `field`, if requested.
fn expect_paired(body: &str, field: Option<&'static str>) -> Result<Vec<u8>, PairError> {
    let doc = parse_root(body)?;
    if child_text(&doc, "paired") != Some("1") {
        return Err(PairError::Rejected(""));
    }
    let Some(field) = field else { return Ok(Vec::new()) };
    let value = child_text(&doc, field).unwrap_or_default();
    hex::decode(value).map_err(|_| PairError::Malformed(field))
}

/// Run the full pairing exchange. Returns the host's certificate (PEM) to pin.
pub async fn pair(nv: &NvClient, ep: &Endpoint<'_>, app_version_major: u32, pin: &str) -> Result<String, PairError> {
    let hash = HashAlgo::for_app_version(app_version_major);
    let salt: [u8; 16] = random();
    let identity = nv.identity();

    // 1. Blocks until the PIN is entered on the host (or it gives up).
    let body = nv
        .get_http(
            ep,
            "pair",
            &[
                ("updateState", "1"),
                ("phrase", "getservercert"),
                ("salt", &hex::encode(salt)),
                ("clientcert", &hex::encode(identity.cert_pem())),
            ],
            Some(PIN_TIMEOUT),
        )
        .await
        .map_err(at("getservercert"))?;
    let server_cert_pem = expect_paired(&body, Some("plaincert"))?;
    if server_cert_pem.is_empty() {
        return Err(PairError::AlreadyInProgress);
    }
    let server_cert = X509::from_pem(&server_cert_pem)?;
    let server_cert_pem = String::from_utf8(server_cert_pem).map_err(|_| PairError::Malformed("plaincert"))?;

    // From here on the host holds partial state for us; clear it if anything fails.
    let result = async {
        let mut hs = Handshake::new(hash, &salt, pin, identity);

        // 2.
        let challenge = hex::encode(hs.client_challenge()?);
        let body = nv.get_http(ep, "pair", &[("updateState", "1"), ("clientchallenge", &challenge)], None)
            .await
            .map_err(at("clientchallenge"))?;
        let challenge_response = expect_paired(&body, Some("challengeresponse"))?;

        // 3.
        let answer = hex::encode(hs.answer_server_challenge(&challenge_response)?);
        let body = nv.get_http(ep, "pair", &[("updateState", "1"), ("serverchallengeresp", &answer)], None)
            .await
            .map_err(at("serverchallengeresp"))?;
        let pairing_secret = expect_paired(&body, Some("pairingsecret"))?;

        // 4.
        let client_secret = hex::encode(hs.verify_server_secret(&server_cert, &pairing_secret)?);
        let body = nv
            .get_http(ep, "pair", &[("updateState", "1"), ("clientpairingsecret", &client_secret)], None)
            .await
            .map_err(at("clientpairingsecret"))?;
        expect_paired(&body, None)?;

        // 5.
        let body = nv
            .get_https(ep, &server_cert_pem, "pair", &[("updateState", "1"), ("phrase", "pairchallenge")])
            .await
            .map_err(at("pairchallenge"))?;
        expect_paired(&body, None)
    }
    .await;

    match result {
        Ok(_) => Ok(server_cert_pem),
        Err(e) => {
            nv.unpair(ep).await;
            Err(e)
        }
    }
}

// ---- Pairing sessions (one per host, polled by the UI) ----

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum PairState {
    Pending,
    Paired,
    Failed { error: String },
}

pub struct PairSession {
    pub pin: String,
    pub state: PairState,
    pub task: tokio::task::AbortHandle,
}

#[derive(Clone, Default)]
pub struct PairSessions(Arc<Mutex<HashMap<String, PairSession>>>);

impl PairSessions {
    pub fn get(&self, host_id: &str) -> Option<(String, PairState)> {
        self.0.lock().unwrap().get(host_id).map(|s| (s.pin.clone(), s.state.clone()))
    }

    pub fn insert(&self, host_id: String, session: PairSession) {
        if let Some(old) = self.0.lock().unwrap().insert(host_id, session) {
            old.task.abort();
        }
    }

    pub fn set_state(&self, host_id: &str, state: PairState) {
        if let Some(s) = self.0.lock().unwrap().get_mut(host_id) {
            s.state = state;
        }
    }

    /// Abort and forget the session; returns whether one was still running.
    pub fn cancel(&self, host_id: &str) -> bool {
        match self.0.lock().unwrap().remove(host_id) {
            Some(s) => {
                s.task.abort();
                matches!(s.state, PairState::Pending)
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays the host's side of steps 2–3, the way Sunshine does it.
    struct FakeHost {
        hash: HashAlgo,
        key: [u8; 16],
        identity: Identity,
        server_secret: [u8; 16],
    }

    impl FakeHost {
        fn new(salt: &[u8; 16], pin: &str) -> Self {
            let hash = HashAlgo::Sha256;
            let mut key = [0u8; 16];
            key.copy_from_slice(&hash.digest(&[salt, pin.as_bytes()])[..16]);
            Self { hash, key, identity: Identity::ephemeral().unwrap(), server_secret: random() }
        }

        fn challenge_response(&self, enc_client_challenge: &[u8]) -> Vec<u8> {
            let client_challenge = aes_ecb(&self.key, enc_client_challenge, Mode::Decrypt).unwrap();
            let sig = self.identity.cert.signature().as_slice();
            let response = self.hash.digest(&[&client_challenge, sig, &self.server_secret]);
            let server_challenge: [u8; 16] = random();
            aes_ecb(&self.key, &[response.as_slice(), &server_challenge].concat(), Mode::Encrypt).unwrap()
        }

        fn pairing_secret(&self) -> Vec<u8> {
            let mut signer = Signer::new(MessageDigest::sha256(), &self.identity.key).unwrap();
            signer.update(&self.server_secret).unwrap();
            [self.server_secret.as_slice(), &signer.sign_to_vec().unwrap()].concat()
        }
    }

    fn run(client_pin: &str, host_pin: &str) -> Result<Vec<u8>, PairError> {
        let client = Identity::ephemeral().unwrap();
        let salt: [u8; 16] = random();
        let host = FakeHost::new(&salt, host_pin);

        let mut hs = Handshake::new(HashAlgo::Sha256, &salt, client_pin, &client);
        let resp = host.challenge_response(&hs.client_challenge()?);
        hs.answer_server_challenge(&resp)?;
        hs.verify_server_secret(&host.identity.cert, &host.pairing_secret())
    }

    #[test]
    fn pairs_with_matching_pin() {
        let secret = run("1234", "1234").unwrap();
        assert!(secret.len() > 16, "client secret + signature");
    }

    #[test]
    fn detects_wrong_pin() {
        assert!(matches!(run("1234", "4321"), Err(PairError::WrongPin)));
    }

    #[test]
    fn aes_ecb_round_trips_and_pads() {
        let key = random::<16>();
        let enc = aes_ecb(&key, &[7u8; 20], Mode::Encrypt).unwrap();
        assert_eq!(enc.len(), 32);
        let dec = aes_ecb(&key, &enc, Mode::Decrypt).unwrap();
        assert_eq!(&dec[..20], &[7u8; 20]);
        assert_eq!(&dec[20..], &[0u8; 12]);
    }

    #[test]
    fn pin_is_four_digits() {
        for _ in 0..100 {
            let pin = generate_pin();
            assert_eq!(pin.len(), 4);
            assert!(pin.chars().all(|c| c.is_ascii_digit()));
        }
    }
}
