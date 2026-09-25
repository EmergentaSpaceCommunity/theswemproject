// One question every walk that opens an App should ask, in one place.
//
// The Apps bridge warns when a View calls a host method before its handshake
// has finished - and then serves the call anyway: the vendored `app-bridge.js`
// wraps each request handler in a function that logs the warning and still
// returns `Y(Z,$)`. So the warning is not a bystander. It is a race that
// happened to be won on this machine: the host had already published the state
// the View asked for, this time. The run that loses it is a View reading state
// that is not there yet, and it will not announce itself as a handshake
// problem - it will look like an empty panel or a stale number.
//
// A walk that saw the warning was lucky, so it fails here rather than on an
// unlucky machine, at an unlucky hour, in someone else's walk.
//
// This module holds no process handlers and launches nothing, so it can be
// imported both by `cdp_browser.mjs` and by the drivers that still carry their
// own CDP plumbing.
export const HANDSHAKE_WARNING = "before ui/notifications/initialized";
/// What our own host says when it turns such a call away. Watched beside the
/// vendored warning because that warning is one upgrade away from being
/// reworded, and a check with a single source of truth outside this
/// repository is a check that can go quiet without anyone deciding to.
export const HANDSHAKE_REFUSAL = "[swem-apps] refused a pre-handshake";

/// The collected console lines that say a View jumped its handshake.
export function handshakeViolations(lines) {
  return lines.filter((line) => line.includes(HANDSHAKE_WARNING) || line.includes(HANDSHAKE_REFUSAL));
}

/// Fails the walk when a View called the host before completing its handshake.
/// `fail` is the driver's own exit path, which differs between the shared
/// browser helper and the self-contained drivers.
export function assertHandshakeOrder(lines, where, fail) {
  const violations = handshakeViolations(lines);
  if (violations.length === 0) return;
  fail(
    `${where}: a View called the host before completing the Apps handshake, and the bridge ` +
    `served the call anyway - this run was lucky, the next one need not be. The View must ` +
    `await app.connect() before its first host call. ${violations.length} such call(s):` +
    violations.map((line) => `\n  ${line.slice(0, 400)}`).join(""),
  );
}
