// Regenerate from frontend/pwa/logo.svg and the app's default theme palette:
// npm install --prefix /tmp/openwebide-icon-tools --no-audit --no-fund @resvg/resvg-js@2.6.2
// NODE_PATH=/tmp/openwebide-icon-tools/node_modules node tools/generate-app-icons.cjs
const { readFileSync, writeFileSync } = require('node:fs');
const { resolve } = require('node:path');
const { Resvg } = require('@resvg/resvg-js');

const root = resolve(__dirname, '..');
const output = resolve(root, 'frontend/pwa');
const logo = readFileSync(resolve(output, 'logo.svg'), 'utf8');
const styles = readFileSync(resolve(root, 'frontend/styles.css'), 'utf8');
const palette = styles.match(/:root\s*\{([^}]+)\}/)[1];
const token = name => palette.match(new RegExp(`--${name}:\\s*(#[0-9a-f]+);`, 'i'))[1];
const artwork = logo.match(/<path\b[^>]+\/>/)[0];

function icon(scale, rounded) {
  const inset = 16 * (1 - scale);
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
  <rect width="32" height="32" rx="${rounded ? 6 : 0}" fill="${token('bg')}"/>
  <g fill="${token('accent')}" transform="translate(${inset} ${inset}) scale(${scale})">${artwork}</g>
</svg>\n`;
}

// All painted geometry fits inside the central 80%-diameter maskable safe circle.
const installed = icon(0.64, false);
for (const size of [180, 192, 512]) {
  const filename = size === 180 ? 'apple-touch-icon.png' : `icon-${size}.png`;
  const png = new Resvg(installed, { fitTo: { mode: 'width', value: size } }).render().asPng();
  writeFileSync(resolve(output, filename), png);
}
const favicon = icon(0.875, true);
writeFileSync(resolve(output, 'favicon.svg'), favicon);
writeFileSync(resolve(output, 'favicon.png'), new Resvg(favicon, { fitTo: { mode: 'width', value: 32 } }).render().asPng());
