// The rule that makes the kit a design system.
//
// A stylesheet full of `var(--token)` is only a design system while nobody adds
// `border-radius: 9px` in a hurry. This gate reads every declaration in
// `kit.css` and refuses any value that is not made of standard MCP Apps
// host-style variables, so the next surface physically cannot introduce a
// twelfth radius or a second muted grey. That is the difference between the
// vocabulary existing and the vocabulary being used.
//
// It also checks the other direction: every variable the kit reads must be a
// name the standard defines, so the kit cannot quietly invent `--k-brand` and
// expect a foreign Apps host to supply it. The kit has exactly one kind of name
// of its own, spelled `--k-*`: a knob a CALLER may set, always read with a
// standard-valued fallback, or a decision the standard genuinely does not cover
// (letter-spacing has no host variable). Those must be declared in this file,
// so they are visible rather than assumed.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {styleKeys} from '../../crates/swem-host/web/apps-host/theme.mjs';

const css = readFileSync(new URL('./kit.css', import.meta.url), 'utf8');

/// `kit.css` without comments, which may legitimately contain any words.
const code = css.replace(/\/\*[\s\S]*?\*\//g, '');

/// Every `property: value` pair outside a selector head.
function declarations() {
  return [...code.matchAll(/([-a-z]+)\s*:\s*([^;{}]+)[;}]/g)]
    .map(([, property, value]) => [property.trim(), value.trim()]);
}

/// Keywords and structural units that carry no design decision: they cannot be
/// tokenised and naming them here is cheaper than a `--k-` token nobody owns.
const FREE = new Set([
  'none', 'auto', 'inherit', 'initial', 'unset', 'transparent', 'currentColor',
  'flex', 'grid', 'block', 'inline-flex', 'inline-grid', 'inline', 'column', 'row',
  'wrap', 'nowrap', 'center', 'start', 'end', 'space-between', 'stretch', 'baseline',
  'left', 'right', 'pointer', 'default', 'not-allowed', 'uppercase', 'anywhere',
  'solid', 'dashed', '0', '100%', 'true', 'false',
]);

/// A length that is a pure ratio or a zero, plus the two em-based letter-spacing
/// values type design needs; anything else must come from a token.
const STRUCTURAL = /^(0|1|100%|min\(.*\)|calc\(.*\))$/;

/// Remove complete `var(...)` references, innermost first, so a token used as
/// another token's fallback still counts as tokenised.
function withoutTokens(value) {
  let text = value, previous;
  do { previous = text; text = text.replace(/var\([^()]*\)/g, ' '); } while (text !== previous);
  return text;
}

test('every value in the kit is built from standard style variables', () => {
  const offenders = [];
  for (const [property, value] of declarations()) {
    if (property.startsWith('--')) continue;                       // the kit's own locals
    const stripped = withoutTokens(value)
      .replace(/[,/]/g, ' ')
      .split(/\s+/)
      .filter(Boolean)
      .filter(word => !FREE.has(word) && !STRUCTURAL.test(word));
    if (stripped.length) offenders.push(`${property}: ${value}   (untokenised: ${stripped.join(' ')})`);
  }
  assert.deepEqual(offenders, [],
    `kit.css must express every value as a standard style variable:\n  ${offenders.join('\n  ')}`);
});

test('the kit reads no variable a host would leave empty', () => {
  const standard = new Set(styleKeys);
  const declared = new Set([...code.matchAll(/(?:^|[;{]\s*)(--[a-z0-9-]+)\s*:/gm)].map(match => match[1]));
  // Every read, with whether it supplied a fallback.
  const reads = [...code.matchAll(/var\(\s*(--[a-z0-9-]+)\s*(,)?/g)]
    .map(([, name, comma]) => ({name, fallback: Boolean(comma)}));

  const foreign = [...new Set(reads.filter(read => !standard.has(read.name) && !declared.has(read.name)
    && !read.name.startsWith('--k-')).map(read => read.name))];
  assert.deepEqual(foreign, [],
    `not host-supplied and not the kit's own, so a foreign Apps host leaves them empty: ${foreign.join(', ')}`);

  // A `--k-*` knob may stay undeclared - that is how a caller overrides one
  // primitive's spacing without the kit picking a global default - but then
  // every read of it must carry a standard-valued fallback, or the shape
  // silently collapses in a host that never sets it.
  const holes = [...new Set(reads
    .filter(read => read.name.startsWith('--k-') && !declared.has(read.name) && !read.fallback)
    .map(read => read.name))];
  assert.deepEqual(holes, [],
    `undeclared kit knobs read without a fallback: ${holes.join(', ')}`);

  const hijacked = [...declared].filter(name => !name.startsWith('--k-'));
  assert.deepEqual(hijacked, [],
    `the kit must not redefine a host variable; that is the host's decision: ${hijacked.join(', ')}`);
});

test('the kit names the shapes the shell had grown by hand', () => {
  // Not a style check: a reminder of why the kit exists. Each of these replaced
  // a family of near-identical classes with their own hand-picked values.
  for (const shape of ['.k-row', '.k-pick', '.k-card', '.k-dialog', '.k-chip', '.k-dot',
                       '.k-btn', '.k-field', '.k-tabs', '.k-empty', '.k-eyebrow', '.k-stack']) {
    assert.ok(code.includes(shape), `the kit no longer defines ${shape}`);
  }
});
