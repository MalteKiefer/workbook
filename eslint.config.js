import js from '@eslint/js';
import globals from 'globals';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  { ignores: ['dist', 'src-tauri', 'docs', 'node_modules'] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2021,
      globals: globals.browser,
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      // eslint-plugin-react-hooks v7's "recommended" config bundles the full
      // React Compiler rule family (set-state-in-effect, purity,
      // immutability, static-components, ...) alongside the two classic
      // hooks rules. This codebase was not written against those Compiler
      // rules, and enabling them wholesale flags many long-standing,
      // correct patterns (e.g. resetting derived state or loading data in
      // a useEffect keyed on a prop/id change) that would need real
      // refactors to "fix" — out of scope for this lint setup. We only
      // enable the two classic rules this project's code and its many
      // narrowly-scoped disable comments actually target.
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],
    },
  },
);
