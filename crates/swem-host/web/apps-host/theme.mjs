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

export function readHostContext() {
  return hostContextFromStyle(getComputedStyle(document.documentElement));
}

export function watchHostContext(bridge) {
  const observer = new MutationObserver(() => bridge.setHostContext(readHostContext()));
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'style', 'class'] });
  return () => observer.disconnect();
}
