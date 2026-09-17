import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import hooks from 'eslint-plugin-react-hooks';
export default tseslint.config(
  { ignores: ['src/ipc/types.gen.ts', 'dist/**'] }, js.configs.recommended,
  ...tseslint.configs.recommendedTypeChecked,
  { files: ['src/**/*.{ts,tsx}'], languageOptions: { parserOptions: {
      project: './tsconfig.json', tsconfigRootDir: import.meta.dirname } },
    plugins: { 'react-hooks': hooks }, rules: {
      ...hooks.configs.recommended.rules,
      '@typescript-eslint/no-floating-promises': 'error',
      '@typescript-eslint/no-misused-promises': 'error',
      '@typescript-eslint/no-explicit-any': 'error' } });
