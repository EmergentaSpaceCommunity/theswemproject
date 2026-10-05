// The presentation contract that crosses the sandbox is the MCP Apps host-style
// vocabulary in full: colour roles, type, weight, rhythm, radius, stroke and
// elevation. `theme.test.mjs` asserts this list equals the extension schema's
// own key set exactly, in both directions, so the host can neither fall behind
// the standard nor invent a SWEM token beside it.
//
// It used to be ten colour variables. The sixty-six that were missing are
// precisely the ones that decide whether a surface looks designed or assembled,
// which is why every SWEM surface used to re-decide them for itself.
export const styleKeys = [
  // colour roles
  '--color-background-primary', '--color-background-secondary', '--color-background-tertiary',
  '--color-background-inverse', '--color-background-ghost', '--color-background-info',
  '--color-background-danger', '--color-background-success', '--color-background-warning',
  '--color-background-disabled', '--color-text-primary', '--color-text-secondary',
  '--color-text-tertiary', '--color-text-inverse', '--color-text-ghost', '--color-text-info',
  '--color-text-danger', '--color-text-success', '--color-text-warning', '--color-text-disabled',
  '--color-border-primary', '--color-border-secondary', '--color-border-tertiary',
  '--color-border-inverse', '--color-border-ghost', '--color-border-info',
  '--color-border-danger', '--color-border-success', '--color-border-warning',
  '--color-border-disabled', '--color-ring-primary', '--color-ring-secondary',
  '--color-ring-inverse', '--color-ring-info', '--color-ring-danger', '--color-ring-success',
  '--color-ring-warning',
  // type
  '--font-sans', '--font-mono', '--font-weight-normal', '--font-weight-medium',
  '--font-weight-semibold', '--font-weight-bold', '--font-text-xs-size', '--font-text-sm-size',
  '--font-text-md-size', '--font-text-lg-size', '--font-heading-xs-size',
  '--font-heading-sm-size', '--font-heading-md-size', '--font-heading-lg-size',
  '--font-heading-xl-size', '--font-heading-2xl-size', '--font-heading-3xl-size',
  '--font-text-xs-line-height', '--font-text-sm-line-height', '--font-text-md-line-height',
  '--font-text-lg-line-height', '--font-heading-xs-line-height', '--font-heading-sm-line-height',
  '--font-heading-md-line-height', '--font-heading-lg-line-height',
  '--font-heading-xl-line-height', '--font-heading-2xl-line-height',
  '--font-heading-3xl-line-height',
  // radius and stroke
  '--border-radius-xs', '--border-radius-sm', '--border-radius-md', '--border-radius-lg',
  '--border-radius-xl', '--border-radius-full', '--border-width-regular',
  // elevation
  '--shadow-hairline', '--shadow-sm', '--shadow-md', '--shadow-lg',
];

export function hostContextFromStyle(style) {
  const variables = Object.fromEntries(styleKeys.map(key => [key, style.getPropertyValue(key).trim()])
    .filter(([, value]) => value));
  return { theme: style.colorScheme === 'light' ? 'light' : 'dark', styles: { variables } };
}

/// Where an App is, in the specification's words: the platform the page is
/// on (the Workbench is `web`, the page inside a messenger `mobile` - the
/// place, never the width), how it is drawn, the room it has, and what the
/// device can do. Width and touch go where the specification puts them.
export function hostContextOfPlace({ platform = 'web', displayMode = 'inline', width, height, touch, hover } = {}) {
  const context = { platform, displayMode, availableDisplayModes: [displayMode] };
  if (Number.isFinite(width) && Number.isFinite(height) && width > 0) {
    context.containerDimensions = displayMode === 'fullscreen' ? { width, height } : { width, maxHeight: height };
  }
  if (typeof touch === 'boolean' && typeof hover === 'boolean') context.deviceCapabilities = { touch, hover };
  return context;
}

function placeOf(container, place) {
  const room = container ? container.getBoundingClientRect() : null;
  const can = (query) => typeof matchMedia === 'function' && matchMedia(query).matches;
  return hostContextOfPlace({
    ...place,
    width: room ? Math.round(room.width) : undefined,
    height: room ? Math.round(place?.displayMode === 'fullscreen' ? room.height : window.innerHeight * 0.6) : undefined,
    touch: can('(pointer: coarse)'),
    hover: can('(hover: hover)'),
  });
}

/// The whole context the host tells an App: the theme and the style
/// variables, and where it is.
export function readHostContext(container, place) {
  return { ...hostContextFromStyle(getComputedStyle(document.documentElement)), ...placeOf(container, place) };
}

/// Tell the App again whenever the theme changes or the room it has does.
export function watchHostContext(bridge, container, place) {
  const tell = () => bridge.setHostContext(readHostContext(container, place));
  const observer = new MutationObserver(tell);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'style', 'class'] });
  const resized = typeof ResizeObserver === 'function' && container ? new ResizeObserver(tell) : null;
  resized?.observe(container);
  return () => {
    observer.disconnect();
    resized?.disconnect();
  };
}
