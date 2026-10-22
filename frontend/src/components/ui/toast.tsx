"use client"

import { Toast as ToastPrimitive } from "@base-ui/react/toast"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, CheckmarkCircle02Icon, InformationCircleIcon } from "@hugeicons/core-free-icons"
import { cn } from "cn"

/**
 * The one place toasts are configured. Everything that needs to tell the user
 * something short goes through `toast`, from React or not:
 *
 *   toast.info('…'); toast.success('…'); toast.error('…', { description: '…' })
 *
 * Position is a single constant below, default bottom right.
 */
const toastManager = ToastPrimitive.createToastManager()

/** Where toasts appear. Change here and every toast in the app follows. */
const VIEWPORT_POSITION = "bottom-4 right-4 items-end"

const TIMEOUT_MS = { info: 5000, success: 3000, error: 8000 } as const
const MAX_VISIBLE = 4

type Kind = keyof typeof TIMEOUT_MS
type Options = { description?: string; timeout?: number }

function show(kind: Kind, title: string, options: Options = {}) {
  return toastManager.add({
    title,
    description: options.description,
    type: kind,
    timeout: options.timeout ?? TIMEOUT_MS[kind],
    // An error interrupts a screen reader; the others wait their turn.
    priority: kind === "error" ? "high" : "low",
  })
}

const toast = {
  info: (title: string, options?: Options) => show("info", title, options),
  success: (title: string, options?: Options) => show("success", title, options),
  error: (title: string, options?: Options) => show("error", title, options),
  close: (id?: string) => toastManager.close(id),
}

const ICONS = { info: InformationCircleIcon, success: CheckmarkCircle02Icon, error: Alert02Icon }
const ICON_COLOR = { info: "text-muted-foreground", success: "text-emerald-600 dark:text-emerald-400", error: "text-destructive" }

function ToastList() {
  const { toasts } = ToastPrimitive.useToastManager()
  return toasts.map((item) => {
    const kind = (item.type && item.type in ICONS ? item.type : "info") as Kind
    return (
      <ToastPrimitive.Root
        key={item.id}
        toast={item}
        data-slot="toast"
        className={cn(
          "pointer-events-auto flex w-80 max-w-[calc(100vw-2rem)] items-start gap-3 rounded-lg bg-popover p-3 text-popover-foreground shadow-lg ring-1 ring-foreground/10 outline-none",
          "transition-all duration-200 data-starting-style:translate-y-2 data-starting-style:opacity-0 data-ending-style:translate-x-4 data-ending-style:opacity-0 data-limited:opacity-0",
        )}
      >
        <HugeiconsIcon icon={ICONS[kind]} className={cn("mt-0.5 size-4 shrink-0", ICON_COLOR[kind])} />
        <ToastPrimitive.Content className="flex min-w-0 flex-1 flex-col gap-0.5">
          <ToastPrimitive.Title className="text-sm font-medium" />
          <ToastPrimitive.Description className="text-xs text-muted-foreground" />
        </ToastPrimitive.Content>
        <ToastPrimitive.Close
          aria-label="关闭提示"
          className="-mt-1 -mr-1 flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-hidden hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          ×
        </ToastPrimitive.Close>
      </ToastPrimitive.Root>
    )
  })
}

/** Mount once, near the app root. */
function Toaster() {
  return (
    <ToastPrimitive.Provider toastManager={toastManager} limit={MAX_VISIBLE}>
      <ToastPrimitive.Portal>
        <ToastPrimitive.Viewport
          data-slot="toast-viewport"
          className={cn("pointer-events-none fixed z-[100] flex flex-col gap-2", VIEWPORT_POSITION)}
        >
          <ToastList />
        </ToastPrimitive.Viewport>
      </ToastPrimitive.Portal>
    </ToastPrimitive.Provider>
  )
}

export { Toaster, toast }
