import { Effect } from "effect"
import type { ExtensionUiBridge } from "./extension-ui.ts"
import type {
  PiAuthEvent,
  PiAuthProvider,
} from "./pi-auth.ts"
import type {
  PiSessionShape,
} from "./pi-session.ts"

const authTypeLabel = (provider: PiAuthProvider): string =>
  provider.type === "oauth"
    ? (provider.loginLabel ?? "Account")
    : "API key"

export const loginProviderLabel = (
  provider: PiAuthProvider,
): string => {
  const status = provider.configured
    ? ` · set${provider.source ? ` via ${provider.source}` : ""}`
    : ""
  return `${provider.name} · ${authTypeLabel(provider)} · ${provider.id}${status}`
}

export const matchingLoginProviders = (
  providers: ReadonlyArray<PiAuthProvider>,
  providerRef: string,
): ReadonlyArray<PiAuthProvider> => {
  const query = providerRef.trim().toLowerCase()
  if (query.length === 0) return providers
  const exact = providers.filter(
    (provider) =>
      provider.id.toLowerCase() === query ||
      provider.name.toLowerCase() === query,
  )
  return exact.length > 0
    ? exact
    : providers.filter((provider) =>
        loginProviderLabel(provider).toLowerCase().includes(query),
      )
}

export const authEventText = (event: PiAuthEvent): string => {
  switch (event.type) {
    case "info": {
      const links = (event.links ?? [])
        .map((link) =>
          link.label === undefined
            ? link.url
            : `${link.label}: ${link.url}`,
        )
        .join("\n")
      return links.length === 0
        ? event.message
        : `${event.message}\n${links}`
    }
    case "auth_url":
      return [
        event.instructions,
        event.url,
      ].filter((part) => part !== undefined).join("\n")
    case "device_code":
      return `Open ${event.verificationUri}\nEnter code: ${event.userCode}`
    case "progress":
      return event.message
  }
}

const chooseProvider = (
  providers: ReadonlyArray<PiAuthProvider>,
  providerRef: string,
  ui: ExtensionUiBridge,
): Effect.Effect<PiAuthProvider | undefined, Error> =>
  Effect.gen(function* () {
    const matches = matchingLoginProviders(providers, providerRef)
    if (matches.length === 1) return matches[0]

    const choices = matches.length > 0 ? matches : providers
    const labels = choices.map(loginProviderLabel)
    const selected = yield* Effect.tryPromise({
      try: () => ui.search("Login", labels, providerRef),
      catch: (cause) =>
        cause instanceof Error ? cause : new Error(String(cause)),
    })
    if (selected === undefined) return undefined
    return choices[labels.indexOf(selected)]
  })

export interface LoginFlowResult {
  readonly provider: PiAuthProvider
  readonly message: string
  readonly loggedIn: boolean
}

export const runLoginFlow = (
  pi: Pick<PiSessionShape, "authProviders" | "login">,
  ui: ExtensionUiBridge,
  providerRef: string,
  signal: AbortSignal,
): Effect.Effect<LoginFlowResult | undefined, unknown> =>
  Effect.gen(function* () {
    const providers = yield* pi.authProviders
    const provider = yield* chooseProvider(providers, providerRef, ui)
    if (provider === undefined) return undefined

    if (!provider.interactive) {
      return {
        provider,
        loggedIn: false,
        message: `${provider.methodName} is set outside Pi for ${provider.name}`,
      }
    }

    const authMessages: Array<string> = []
    yield* pi.login(provider.id, provider.type, {
      signal,
      prompt: (prompt) =>
        ui.authPrompt(
          prompt,
          authMessages.length === 0
            ? undefined
            : authMessages.join("\n\n"),
        ),
      notify: (event) => {
        const message = authEventText(event)
        if (authMessages.at(-1) !== message) {
          authMessages.push(message)
          if (authMessages.length > 6) authMessages.shift()
        }
        ui.setAuthNotice(message)
      },
    })

    return {
      provider,
      loggedIn: true,
      message:
        provider.type === "oauth"
          ? `Logged in to ${provider.name}`
          : `Saved API key for ${provider.name}`,
    }
  }).pipe(
    Effect.ensuring(
      Effect.sync(() => ui.setAuthNotice(undefined)),
    ),
  )
