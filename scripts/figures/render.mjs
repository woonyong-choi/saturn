// docs/assets의 *.dap마다 라이트와 다크 SVG를 만든다. 문서의 <picture>가 둘 중 하나를 고른다.
// 사용: DAPHNIS_PATH=<daphnis 작업본> node scripts/figures/render.mjs [원본.dap ...]
// 결과: docs/assets/<이름>.<언어>.light.svg, docs/assets/<이름>.<언어>.dark.svg
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const assets = join(root, 'docs/assets');
const daphnis = process.env.DAPHNIS_PATH;
if (!daphnis) {
  console.error('DAPHNIS_PATH에 daphnis 작업본 경로를 지정한다.');
  process.exit(2);
}
const load = (path) => import(pathToFileURL(join(daphnis, path)).href);
const { buildFigure } = await load('src/build.js');
const { toSvg } = await load('src/svg.js');
const { themedSvgs } = await load('scripts/build-showcase.mjs');

const args = process.argv.slice(2);
const sources = args.length ? args.map((a) => resolve(a)) : readdirSync(assets).filter((f) => f.endsWith('.dap')).map((f) => join(assets, f));

for (const source of sources) {
  const result = await buildFigure(readFileSync(source, 'utf8'), { baseDir: dirname(source), strict: true });
  const name = basename(source, '.dap');
  const { light, dark } = themedSvgs(await toSvg(result, { isStatic: false, name }));
  writeFileSync(join(dirname(source), `${name}.light.svg`), light);
  writeFileSync(join(dirname(source), `${name}.dark.svg`), dark);
}
