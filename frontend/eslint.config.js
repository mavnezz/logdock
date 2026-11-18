import { readdirSync } from 'node:fs'

import skipFormatting from '@vue/eslint-config-prettier/skip-formatting'
import { defineConfigWithVueTs, vueTsConfigs } from '@vue/eslint-config-typescript'
import { createTypeScriptImportResolver } from 'eslint-import-resolver-typescript'
import { importX } from 'eslint-plugin-import-x'
import pluginVue from 'eslint-plugin-vue'

// Dependencies point inward: app → features → shared.
// Features do not import each other (docs/architecture.md, 10).
const features = readdirSync('src/features', { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name)

const zones = [
  {
    target: './src/shared',
    from: ['./src/features', './src/app'],
    message: 'shared/ must not depend on features or the app shell.',
  },
  {
    target: './src/features',
    from: './src/app',
    message: 'Features know nothing about the app shell.',
  },
  ...features.map((feature) => ({
    target: `./src/features/${feature}`,
    from: './src/features',
    except: [`./${feature}`],
    message: 'Features must not import each other; move shared code to shared/.',
  })),
]

export default defineConfigWithVueTs(
  {
    name: 'logdock/ignores',
    ignores: ['dist/', 'public/', 'coverage/', 'src/shared/api/schema.d.ts'],
  },
  pluginVue.configs['flat/recommended'],
  vueTsConfigs.recommendedTypeChecked,
  {
    name: 'logdock/vue',
    rules: {
      // Optional props in TypeScript are explicitly `undefined`; a default adds nothing.
      'vue/require-default-prop': 'off',
    },
  },
  {
    // UI building blocks deliberately have short names, like the elements they wrap.
    name: 'logdock/ui-names',
    files: ['src/shared/ui/**/*.vue'],
    rules: { 'vue/multi-word-component-names': 'off' },
  },
  {
    name: 'logdock/boundaries',
    files: ['src/**/*.{ts,vue}'],
    plugins: { 'import-x': importX },
    settings: {
      'import-x/resolver-next': [
        createTypeScriptImportResolver({ project: './tsconfig.app.json' }),
      ],
    },
    rules: {
      'import-x/no-restricted-paths': ['error', { zones }],
    },
  },
  skipFormatting,
)
