// Minimal History-API router. The bridge serves index.html for unknown paths,
// so deep links like /hosts/abc/apps survive a reload.

export type Route =
  | { name: 'hosts' }
  | { name: 'apps'; hostId: string }
  | { name: 'stream'; hostId: string; appId: string }
  | { name: 'settings' }
  | { name: 'not-found' };

function parse(path: string): Route {
  const parts = path.split('/').filter(Boolean).map(decodeURIComponent);
  if (parts.length === 0 || (parts.length === 1 && parts[0] === 'hosts')) return { name: 'hosts' };
  if (parts[0] === 'settings' && parts.length === 1) return { name: 'settings' };
  if (parts[0] === 'hosts' && parts[2] === 'apps' && parts.length === 3) return { name: 'apps', hostId: parts[1] };
  if (parts[0] === 'hosts' && parts[2] === 'stream' && parts.length === 4)
    return { name: 'stream', hostId: parts[1], appId: parts[3] };
  return { name: 'not-found' };
}

class Router {
  path = $state(location.pathname);
  route = $derived(parse(this.path));

  constructor() {
    addEventListener('popstate', () => (this.path = location.pathname));
  }

  go(path: string, replace = false) {
    if (path === this.path) return;
    history[replace ? 'replaceState' : 'pushState'](null, '', path);
    this.path = path;
    scrollTo(0, 0);
  }
}

export const router = new Router();

/** Use on <a> elements: intercepts plain left-clicks and routes client-side. */
export function link(node: HTMLAnchorElement) {
  const onClick = (e: MouseEvent) => {
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    e.preventDefault();
    router.go(node.pathname);
  };
  node.addEventListener('click', onClick);
  return { destroy: () => node.removeEventListener('click', onClick) };
}
