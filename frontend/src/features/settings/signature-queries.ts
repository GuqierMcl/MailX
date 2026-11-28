import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { SaveSignatureRequest, SetAccountSignatureRequest, SetDefaultSignatureRequest, SignaturesState } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { signatureKeys } from '@/lib/query-client'

/** Reads global signatures and per-account defaults, staying synced across windows. */
export function useSignatures(client: MailxClient) {
  const queryClient = useQueryClient()
  useEffect(() => client.onSignaturesChanged((next) => {
    queryClient.setQueryData(signatureKeys.state(), next)
  }), [client, queryClient])
  return useQuery({
    queryKey: signatureKeys.state(),
    queryFn: () => client.listSignatures(),
  })
}

export function useSaveSignature(client: MailxClient) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (request: SaveSignatureRequest) => client.saveSignature(request),
    onSuccess: (next: SignaturesState) => {
      queryClient.setQueryData(signatureKeys.state(), next)
    },
  })
}

export function useDeleteSignature(client: MailxClient) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => client.deleteSignature(id),
    onSuccess: (next: SignaturesState) => {
      queryClient.setQueryData(signatureKeys.state(), next)
    },
  })
}

export function useSetDefaultSignature(client: MailxClient) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (request: SetDefaultSignatureRequest) => client.setDefaultSignature(request),
    onSuccess: (next: SignaturesState) => {
      queryClient.setQueryData(signatureKeys.state(), next)
    },
  })
}

export function useSetAccountSignature(client: MailxClient) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (request: SetAccountSignatureRequest) => client.setAccountDefaultSignature(request),
    onSuccess: (next: SignaturesState) => {
      queryClient.setQueryData(signatureKeys.state(), next)
    },
  })
}
