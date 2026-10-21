import { createContext, useContext, type ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { MailAddress } from '@/generated/accounts'
import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar'
import type { MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import { avatarHue, avatarImageClass, initials } from './format'

type AvatarSource = { client: MailxClient; enabled: boolean }

const AvatarSourceContext = createContext<AvatarSource | null>(null)

/** Lets every `SenderAvatar` below look up real avatars when the user opted in. */
export function AvatarSourceProvider({ client, enabled, children }: AvatarSource & { children: ReactNode }) {
  return <AvatarSourceContext.Provider value={{ client, enabled }}>{children}</AvatarSourceContext.Provider>
}

/** `null` until a real avatar is known; a lookup failure just keeps the generated one. */
function useAvatar(accountId: string, address: string) {
  const source = useContext(AvatarSourceContext)
  const normalized = address.trim().toLowerCase()
  const query = useQuery({
    queryKey: ['avatar', normalized],
    queryFn: () => source!.client.getAvatar(accountId, normalized),
    enabled: Boolean(source?.enabled) && normalized.includes('@'),
    staleTime: 60 * 60 * 1000,
    gcTime: 60 * 60 * 1000,
  })
  return query.data
    ? { url: `data:${query.data.mimeType};base64,${query.data.data}`, kind: query.data.kind }
    : null
}

/** A stable color per sender, so the same person looks the same everywhere. */
export function SenderAvatar({ sender, accountId, size = 'default', className }: {
  sender: MailAddress
  accountId: string
  size?: 'default' | 'sm' | 'lg'
  className?: string
}) {
  const avatar = useAvatar(accountId, sender.address)
  return (
    <Avatar size={size} className={className}>
      {avatar && <AvatarImage src={avatar.url} alt="" className={avatarImageClass(avatar.kind)} />}
      <AvatarFallback
        className={cn('text-white')}
        style={{ backgroundColor: `hsl(${avatarHue(sender.address)} 45% 42%)` }}
      >
        {initials(sender)}
      </AvatarFallback>
    </Avatar>
  )
}
