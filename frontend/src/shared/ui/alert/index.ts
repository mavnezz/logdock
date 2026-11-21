import { cva, type VariantProps } from 'class-variance-authority'

export { default as Alert } from './Alert.vue'

export const alertVariants = cva(
  'relative flex w-full items-start gap-3 rounded-xl border px-4 py-3 text-sm [&>svg]:mt-0.5 [&>svg]:size-4 [&>svg]:shrink-0',
  {
    variants: {
      variant: {
        default: 'bg-card text-card-foreground shadow-xs',
        success: 'border-success/30 bg-success/8 text-foreground [&>svg]:text-success',
        warning: 'border-warning/35 bg-warning/8 text-foreground [&>svg]:text-warning',
        destructive: 'border-destructive/40 bg-destructive/5 text-destructive [&>svg]:text-current',
      },
    },
    defaultVariants: { variant: 'default' },
  },
)

export type AlertVariants = VariantProps<typeof alertVariants>
