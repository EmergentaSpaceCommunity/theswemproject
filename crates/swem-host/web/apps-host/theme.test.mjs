// The design system's only mechanical guardrail. It checks three things, and
// each one failed silently before it existed in this form:
//
//   1. the host speaks the standard's whole style vocabulary and nothing else;
//   2. both shipped palettes actually define every one of those names;
//   3. readable text clears 4.5:1 on every surface it is drawn on.
//
// (1) is what stops the design system drifting into a private SWEM ontology in
// either direction - falling behind the standard, or inventing tokens beside
// it. (2) is what stops a surface from quietly inheriting a browser default
// because nobody declared a font size. Together they are the reason a new
// surface no longer has to decide type, rhythm, radius and elevation for
// itself.
import test from 'node:test';
import assert from 'node:assert/strict';
import {hostContextFromStyle, hostContextOfPlace, styleKeys} from './theme.mjs';
import {McpUiHostContextSchema, McpUiHostStylesSchema} from '@modelcontextprotocol/ext-apps';
import {readFileSync} from 'node:fs';

/// The extension's own key set, read out of its schema rather than copied.
function schemaStyleKeys() {
  let variables = McpUiHostStylesSchema._zod.def.shape.variables;
  while (variables?._zod?.def?.innerType) variables = variables._zod.def.innerType;
  const key = variables._zod.def.keyType._zod.def;
  return key.entries ? Object.values(key.entries)
    : key.options.map(option => option._zod.def.values?.[0] ?? option._zod.def.value);
}

/// Every `--name: value` pair of one CSS block, values kept verbatim.
function declarations(block) {
  return Object.fromEntries([...block.matchAll(/(--[\w-]+)\s*:\s*([^;]+)/g)]
    .map(match => [match[1], match[2].trim()]));
}

/// SWEM's palettes, read from the one file that holds them: the bare `:root`
/// blocks together are the dark theme plus the theme-independent half, and
/// `:root[data-theme=light]` overrides it. The shell and both domain Apps
/// inline this same file, so there is nothing else to check.
function palettes() {
  const css = readFileSync(new URL('../../../../web/view-kit/palette.css', import.meta.url), 'utf8');
  const blocks = [...css.matchAll(/:root(\[data-theme=light\])?\s*\{([^}]+)\}/g)];
  const dark = {}, light = {};
  for (const [, isLight, body] of blocks) Object.assign(isLight ? light : dark, declarations(body));
  return {dark, light: {...dark, ...light}};
}

test('the host exports the standard style vocabulary, whole and with nothing added', () => {
  const standard = schemaStyleKeys();
  assert.equal(new Set(styleKeys).size, styleKeys.length, 'duplicate key in styleKeys');
  assert.deepEqual([...styleKeys].sort(), [...standard].sort(),
    'styleKeys must equal the MCP Apps schema key set exactly - no SWEM token, no missing standard token');
});

test('a host context carrying every variable still validates against the schema', () => {
  const complete = hostContextFromStyle({colorScheme: 'light', getPropertyValue: () => '#123456'});
  assert.equal(Object.keys(complete.styles.variables).length, styleKeys.length);
  assert.deepEqual(JSON.parse(JSON.stringify(McpUiHostContextSchema.parse(complete))), complete);
});

test('only declared values cross the boundary, never arbitrary CSS data', () => {
  for (const theme of ['light', 'dark']) {
    const context = hostContextFromStyle({colorScheme: theme,
      getPropertyValue: key => key === '--color-text-primary' ? ' #123456 ' : ''});
    assert.deepEqual(context, {theme, styles: {variables: {'--color-text-primary': '#123456'}}});
  }
});

test('both shipped palettes define every variable the host promises to send', () => {
  const {dark, light} = palettes();
  for (const [name, declared] of [['dark', dark], ['light', light]]) {
    const missing = styleKeys.filter(key => !(key in declared));
    assert.deepEqual(missing, [], `${name} palette declares no value for: ${missing.join(', ')}`);
  }
});

test('both shipped palettes keep readable text above the contrast floor on every surface', () => {
  // `ghost`, `disabled` and `inverse` are deliberately outside this floor and
  // the reason is not laziness: inverse text is drawn on the inverse surface
  // rather than these three, and de-emphasised roles lose their meaning the
  // moment they are forced to the same contrast as body text. Every role a
  // user is expected to READ is checked.
  const readable = ['primary', 'secondary', 'tertiary', 'danger', 'warning', 'info', 'success'];
  const surfaces = ['primary', 'secondary', 'tertiary'];
  const luminance = hex => {
    const rgb = hex.match(/[a-f\d]{2}/gi).map(byte => parseInt(byte, 16) / 255)
      .map(value => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
    return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
  };
  const ratio = (a, b) => (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
  for (const [name, declared] of Object.entries(palettes())) {
    for (const role of readable) {
      for (const surface of surfaces) {
        const text = declared[`--color-text-${role}`], background = declared[`--color-background-${surface}`];
        assert.ok(ratio(luminance(text), luminance(background)) >= 4.5,
          `${name}: text-${role} on background-${surface} is ${ratio(luminance(text), luminance(background)).toFixed(2)}:1`);
      }
    }
    // A semantic fill is only usable if the inverse text it carries is legible.
    for (const role of ['danger', 'warning', 'info', 'success']) {
      const fill = declared[`--color-background-${role}`], text = declared['--color-text-inverse'];
      assert.ok(ratio(luminance(fill), luminance(text)) >= 4.5,
        `${name}: text-inverse on background-${role} is ${ratio(luminance(fill), luminance(text)).toFixed(2)}:1`);
    }
  }
});

test('the shell decides colour, radius, stroke and type only in its palette', () => {
  // The palettes are where values belong; every rule that composes a screen
  // must reach for a name instead. Before the vocabulary existed the shell
  // carried five hand-picked radii, three mono font declarations and two pure
  // black modal shadows that were simply wrong in the light theme, because
  // there was nothing to reach for.
  //
  // Spacing is deliberately not covered: the MCP Apps vocabulary has no spacing
  // scale, so px paddings and layout geometry stay literal here. The kit sets
  // its own rhythm from the type sizes, which is the convention new surfaces
  // follow.
  const css = readFileSync(new URL('../../src/workbench_shell/shell.html', import.meta.url), 'utf8');
  const style = [...css.matchAll(/<style>([\s\S]*?)<\/style>/g)].map(match => match[1]).join('\n');
  const composed = style
    .replace(/:root(\[data-theme=light\])?\s*\{[^}]*\}/g, '')
    .replace(/\/\*[\s\S]*?\*\//g, '');
  const offences = [
    ...[...composed.matchAll(/#[0-9a-fA-F]{3,8}\b/g)].map(m => `colour literal ${m[0]}`),
    ...[...composed.matchAll(/border-radius:[^;}]*\d+(px|%)/g)].map(m => `radius literal in "${m[0]}"`),
    ...[...composed.matchAll(/\b\d+px solid\b/g)].map(m => `stroke literal "${m[0]}"`),
    ...[...composed.matchAll(/font-size:\s*[\d.]+(px|rem)/g)].map(m => `type literal in "${m[0]}"`),
  ];
  assert.deepEqual(offences, [],
    `the shell must name these, not pick them:\n  ${offences.join('\n  ')}`);
});

test('the shell inlines the shared palette and kit rather than restating them', () => {
  const css = readFileSync(new URL('../../src/workbench_shell/shell.html', import.meta.url), 'utf8');
  for (const slot of ['__PALETTE_CSS__', '__KIT_CSS__']) {
    assert.ok(css.includes(slot), `the shell no longer reserves a slot for ${slot}`);
  }
});

// Where an App is told it is: the place, how it is drawn, the room, the
// device - each in the extension's own words, and validated by its schema.
test('the host says where an App is, in the standard\'s words', () => {
  const mobile = hostContextOfPlace({platform: 'mobile', displayMode: 'fullscreen', width: 390, height: 700, touch: true, hover: false});
  assert.equal(mobile.platform, 'mobile');
  assert.equal(mobile.displayMode, 'fullscreen');
  assert.deepEqual(mobile.availableDisplayModes, ['fullscreen']);
  assert.deepEqual(mobile.containerDimensions, {width: 390, height: 700});
  assert.deepEqual(mobile.deviceCapabilities, {touch: true, hover: false});
  const beside = hostContextOfPlace({platform: 'web', displayMode: 'inline', width: 320, height: 500, touch: false, hover: true});
  assert.deepEqual(beside.containerDimensions, {width: 320, maxHeight: 500});
  // Nothing of it is outside the standard's own shape.
  for (const context of [mobile, beside, hostContextOfPlace()]) McpUiHostContextSchema.parse(context);
  // Undeclared room and device are left out, not invented.
  assert.equal(hostContextOfPlace().containerDimensions, undefined);
  assert.equal(hostContextOfPlace().deviceCapabilities, undefined);
});
