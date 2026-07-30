import type { ModelRuntime } from "@earendil-works/pi-coding-agent"

export type PiAuthType = Parameters<ModelRuntime["login"]>[1]
export type PiAuthInteraction = Parameters<ModelRuntime["login"]>[2]
export type PiAuthPrompt = Parameters<PiAuthInteraction["prompt"]>[0]
export type PiAuthEvent = Parameters<PiAuthInteraction["notify"]>[0]

export interface PiAuthProvider {
  readonly id: string
  readonly name: string
  readonly type: PiAuthType
  readonly methodName: string
  readonly loginLabel: string | undefined
  readonly interactive: boolean
  readonly configured: boolean
  readonly source: string | undefined
}

export const piAuthProviders = (
  runtime: Pick<
    ModelRuntime,
    "getProviders" | "getProviderAuthStatus"
  >,
): ReadonlyArray<PiAuthProvider> =>
  runtime
    .getProviders()
    .flatMap((provider): Array<PiAuthProvider> => {
      const status = runtime.getProviderAuthStatus(provider.id)
      const common = {
        id: provider.id,
        name: provider.name,
        configured: status.configured,
        source: status.configured
          ? (status.label ?? status.source)
          : undefined,
      }
      const methods: Array<PiAuthProvider> = []
      if (provider.auth.oauth !== undefined) {
        methods.push({
          ...common,
          type: "oauth",
          methodName: provider.auth.oauth.name,
          loginLabel: provider.auth.oauth.loginLabel,
          interactive: true,
        })
      }
      if (provider.auth.apiKey !== undefined) {
        methods.push({
          ...common,
          type: "api_key",
          methodName: provider.auth.apiKey.name,
          loginLabel: undefined,
          interactive: provider.auth.apiKey.login !== undefined,
        })
      }
      return methods
    })
    .toSorted(
      (left, right) =>
        left.name.localeCompare(right.name) ||
        left.type.localeCompare(right.type),
    )
