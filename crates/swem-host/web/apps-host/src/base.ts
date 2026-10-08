// Where the Workbench is drawn. Started by its own command it is at the
// root of its address; built into a product it is under a path of that
// product's server. The page finds out from where it was opened, and asks
// its host beside itself.

const opened = typeof document === "undefined" ? "/" : new URL(".", document.baseURI).pathname;

/// The path the Workbench is drawn under, without its last stroke.
export const UNDER = opened.replace(/\/+$/, "");

/// An address of the host, from the root, as it is where the Workbench is drawn.
export const at = (path: string, under: string = UNDER): string => (path.startsWith("/") ? `${under}${path}` : path);

/// Drawn under `/peers/<id>/` of another host of the person's: this page is
/// that host's, shown inside the other's, and draws no rail of its own.
export const WITHIN_A_HOST = /\/peers\/[^/]+$/.test(UNDER);
