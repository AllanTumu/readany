// What the npm package actually exposes, exercised through the built package.
//
// The binding had no test of any kind, and went two releases' worth of API
// behind without anything failing: it was still serialising a page count where
// the crate had grown per-page provenance, and still flattening every typed
// refusal to an English string. `cargo check` on the root crate passes whatever
// the binding does, so nothing was watching the one artefact npm receives.
//
//   ./scripts/build-wasm.sh && node wasm/test/binding.test.js
//
const assert = require('assert');
const r = require('../../pkg');

let failures = 0;
function check(name, fn) {
  try { fn(); console.log('  ok   ' + name); }
  catch (e) { failures++; console.log('  FAIL ' + name + ' -- ' + e.message); }
}

console.log('version:', r.version());
check('version matches the crate', () => assert.strictEqual(r.version(), '0.2.0'));

// --- CSV, which is the case that needs `filename` -------------------------
const csv = Buffer.from('amount\n1200\n');
check('a one-column CSV is unknown without a name', () => {
  const p = r.inspect(csv);
  assert.strictEqual(p.kind, 'unknown');
});
check('a one-column CSV routes when named', () => {
  const p = r.inspect(csv, { filename: 'export.csv' });
  assert.strictEqual(p.kind, 'office');
  assert.strictEqual(p.format, 'Csv');
});
check('and reads', () => {
  const doc = r.read(csv, { filename: 'export.csv' });
  assert.ok(doc.markdown.includes('1200'), doc.markdown);
  assert.strictEqual(doc.pages[0].origin, 'text');
  assert.strictEqual(doc.complete, true);
  assert.strictEqual(doc.needs_a_person, false);
  assert.deepStrictEqual(doc.human_pages, []);
  assert.ok(doc.receipt.includes('marked for a person'), doc.receipt);
});

// --- per-page provenance --------------------------------------------------
check('pages carry origin, confidence and human_regions', () => {
  const doc = r.read(csv, { filename: 'export.csv' });
  const p = doc.pages[0];
  assert.strictEqual(typeof p.number, 'number');
  assert.strictEqual(typeof p.markdown, 'string');
  assert.strictEqual(p.origin, 'text');
  assert.strictEqual(p.confidence, undefined);
  assert.deepStrictEqual(p.human_regions, []);
});

// --- an image: unresolved, never silently dropped -------------------------
// 1x1 PNG
const png = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
  'base64');
check('an image is reported unresolved, not dropped', () => {
  const doc = r.read(png);
  assert.strictEqual(doc.route, 'image');
  assert.deepStrictEqual(doc.unresolved_pages, [1]);
  assert.strictEqual(doc.complete, false);
  assert.strictEqual(doc.pages[0].origin, 'unresolved');
});

// --- typed refusals -------------------------------------------------------
check('too_large carries kind, limit, allowed, found', () => {
  const big = Buffer.alloc(2048);
  try {
    r.read(big, { limits: { input_bytes: 1024 } });
    throw new Error('should have refused');
  } catch (e) {
    assert.strictEqual(e.name, 'ReadError');
    assert.strictEqual(e.kind, 'too_large');
    assert.strictEqual(e.limit, 'input size');
    assert.strictEqual(e.allowed, 1024);
    assert.strictEqual(e.found, 2048);
  }
});
check('unsupported is a distinct kind', () => {
  try {
    r.read(Buffer.from('just some prose, not a document'));
    throw new Error('should have refused');
  } catch (e) {
    assert.strictEqual(e.kind, 'unsupported');
  }
});
check('strict turns a partial read into a typed throw', () => {
  try {
    r.read(png, { strict: true });
    throw new Error('should have refused');
  } catch (e) {
    assert.strictEqual(e.kind, 'ocr_required');
    assert.strictEqual(e.pages, 1);
  }
});

// --- option hygiene -------------------------------------------------------
check('a mistyped option is an error, not a shrug', () => {
  try {
    r.read(csv, { filename: 'x.csv', pageMarkers: true });
    throw new Error('should have refused');
  } catch (e) {
    assert.ok(/unknown read option "pageMarkers"/.test(e.message), e.message);
  }
});
check('a mistyped ceiling inside limits is an error too', () => {
  try {
    r.read(csv, { filename: 'x.csv', limits: { input_byte: 10 } });
    throw new Error('should have refused');
  } catch (e) {
    assert.ok(/unknown limits option "input_byte"/.test(e.message), e.message);
  }
});
check('limits and no_limits together are refused', () => {
  try {
    r.read(csv, { no_limits: true, limits: { pages: 5 } });
    throw new Error('should have refused');
  } catch (e) {
    assert.ok(/not both/.test(e.message), e.message);
  }
});
check('a negative ceiling is refused rather than saturating to zero', () => {
  try {
    r.read(csv, { limits: { input_bytes: -1 } });
    throw new Error('should have refused');
  } catch (e) {
    assert.ok(/non-negative/.test(e.message), e.message);
  }
});

// --- page markers ---------------------------------------------------------
check('page_markers works', () => {
  const doc = r.read(csv, { filename: 'export.csv', page_markers: true });
  assert.ok(doc.markdown.startsWith('<!-- Page 1 -->'), doc.markdown.slice(0, 40));
});

// --- defaultLimits --------------------------------------------------------
check('defaultLimits states the contract', () => {
  const l = r.defaultLimits();
  assert.strictEqual(l.input_bytes, 50 * 1024 * 1024);
  assert.strictEqual(l.pages, 200);
  assert.strictEqual(l.pixels_per_page, 40000000);
  assert.strictEqual(l.archive_depth, 2);
});

// --- prepare --------------------------------------------------------------
check('prepare straightens a page without a model', () => {
  const page = r.prepare(png, { profile: 'rendered_page' });
  assert.strictEqual(page.width, 1);
  assert.strictEqual(page.height, 1);
  assert.ok(page.pixels instanceof Uint8Array);
  assert.strictEqual(page.pixels.length, 1);
  assert.strictEqual(page.rotation, 0);
});
check('an unknown profile is named, not defaulted', () => {
  try {
    r.prepare(png, { profile: 'photo' });
    throw new Error('should have refused');
  } catch (e) {
    assert.ok(/unknown profile/.test(e.message), e.message);
  }
});

// --- toMarkdown -----------------------------------------------------------
check('toMarkdown still takes one argument', () => {
  assert.ok(r.toMarkdown(csv, { filename: 'export.csv' }).includes('1200'));
});

console.log(failures === 0 ? '\nALL PASS' : `\n${failures} FAILED`);
process.exit(failures === 0 ? 0 : 1);
