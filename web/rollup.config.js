import svelte from 'rollup-plugin-svelte';
import { terser } from 'rollup-plugin-terser';
import css from 'rollup-plugin-css-only';
import resolve from '@rollup/plugin-node-resolve';

const input = [
  'src/Main.svelte'
];

export default [
  {
    input,
    output: {
      dir: '../target/'
    },
    plugins: [
      svelte({
        compilerOptions: {
          hydratable: true
        }
      }),
      resolve(),
      css({ output: '[name].css' }),
      terser()
    ]
  },
  {
    input,
    output: {
      dir: '../target/ssr/',
      entryFileNames: '[name].mjs',
      preserveModules: true,
      preserveModulesRoot: 'src'
    },
    plugins: [
      svelte({
        compilerOptions: {
          generate: 'ssr',
          hydratable: true,
          css: false // discard style
        }
      }),
      resolve()
    ]
  }
]
