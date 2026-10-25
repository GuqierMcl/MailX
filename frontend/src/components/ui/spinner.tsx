import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { cn } from "cn"
import { HugeiconsIcon } from "@hugeicons/react"
import { Loading03Icon } from "@hugeicons/core-free-icons"

function Spinner({ className, ...props }: Omit<React.ComponentProps<"svg">, "strokeWidth">) {
  useTranslation()
  return (
    <HugeiconsIcon icon={Loading03Icon} strokeWidth={2} data-slot="spinner" role="status" aria-label={t('common:loading')} className={cn("size-4 animate-spin", className)} {...props} />
  )
}

export { Spinner }
