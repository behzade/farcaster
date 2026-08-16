import { mkdtemp, rm } from "node:fs/promises";
import { createConnection, createServer, type Server, type Socket } from "node:net";
import { join } from "node:path";
import { Effect, Exit, Queue, Schema, Scope } from "effect";
import { normalizeNetworkHost } from "./io-permissions.ts";

const MAX_HANDSHAKE_BYTES = 64 * 1024;
const MAX_OPEN_SOCKETS = 512;
const HANDSHAKE_TIMEOUT_MS = 15_000;
const CONNECT_TIMEOUT_MS = 15_000;
const IDLE_TIMEOUT_MS = 5 * 60_000;

export class NativeNetworkProxyError extends Schema.TaggedError<NativeNetworkProxyError>()(
	"NativeNetworkProxyError",
	{ message: Schema.String, cause: Schema.optional(Schema.Defect()) },
) {}

const proxyError = (cause: unknown) => new NativeNetworkProxyError({
	message: cause instanceof Error ? cause.message : String(cause),
	cause,
});

export interface NativeNetworkProxy {
	readonly port: number;
	readonly socketPath: string;
	close(): Promise<void>;
}

class NativeNetworkProxyResource implements NativeNetworkProxy {
	readonly port: number;
	readonly socketPath: string;
	readonly #tcp: Server;
	readonly #unix: Server;
	readonly #clients: Set<Socket>;
	#ownerScope?: Scope.Closeable;
	#closed = false;
	constructor(port: number, socketPath: string, tcp: Server, unix: Server, clients: Set<Socket>) {
		this.port = port;
		this.socketPath = socketPath;
		this.#tcp = tcp;
		this.#unix = unix;
		this.#clients = clients;
	}

	setOwnerScope(scope: Scope.Closeable): void { this.#ownerScope = scope; }

	readonly closeEffect = Effect.fn("NativeNetworkProxy.close")(() => {
		if (this.#closed) return Effect.void;
		this.#closed = true;
		const self = this;
		return Effect.gen(function* () {
			yield* Effect.sync(() => { for (const socket of self.#clients) socket.destroy(); });
			yield* closeServer(self.#tcp);
			yield* closeServer(self.#unix);
		});
	});

	/** Promise boundary adapter. */
	close(): Promise<void> {
		const scope = this.#ownerScope;
		this.#ownerScope = undefined;
		return Effect.runPromise(scope ? Scope.close(scope, Exit.void) : this.closeEffect());
	}
}

/** Scoped proxy acquisition for internal composition. */
export const acquireNativeNetworkProxy = Effect.fn("NativeNetworkProxy.acquire")(function* (hosts: readonly string[]) {
	const allowed = yield* Effect.try({
		try: () => new Set(hosts.map(normalizeNetworkHost)),
		catch: proxyError,
	});
	if (allowed.size === 0) return yield* Effect.fail(proxyError("A native network proxy needs at least one host"));

	// `/tmp` canonicalizes to `/private/tmp` on macOS and keeps Unix socket paths short.
	const directory = yield* Effect.acquireRelease(
		Effect.tryPromise({ try: () => mkdtemp(join("/tmp", "pi-native-proxy-")), catch: proxyError }),
		(path) => Effect.promise(() => rm(path, { recursive: true, force: true })),
	);
	const socketPath = join(directory, "proxy.sock");
	const clients = new Set<Socket>();
	const accepted = yield* Queue.unbounded<Socket>();
	const accept = (socket: Socket) => {
		if (clients.size >= MAX_OPEN_SOCKETS) { socket.destroy(); return; }
		clients.add(socket);
		if (!Queue.offerUnsafe(accepted, socket)) { clients.delete(socket); socket.destroy(); }
	};
	const tcp = yield* Effect.acquireRelease(Effect.sync(() => createServer(accept)), closeServer);
	const unix = yield* Effect.acquireRelease(Effect.sync(() => createServer(accept)), closeServer);
	yield* listen(tcp, { host: "127.0.0.1", port: 0 });
	yield* listen(unix, { path: socketPath });
	const address = tcp.address();
	if (!address || typeof address === "string") return yield* Effect.fail(proxyError("Native proxy has no TCP port"));

	const serve = Effect.forever(
		Queue.take(accepted).pipe(
			Effect.flatMap((socket) => handleAccepted(socket, allowed, clients).pipe(
				Effect.catchCause(() => Effect.sync(() => socket.destroy())),
				Effect.forkScoped,
			)),
		),
	);
	yield* Effect.forkScoped(serve);
	const resource = new NativeNetworkProxyResource(address.port, socketPath, tcp, unix, clients);
	// This finalizer runs before the accept fiber/server/temp-directory finalizers.
	yield* Effect.addFinalizer(() => resource.closeEffect());
	return resource;
});

/** Promise boundary adapter retained for existing callers. */
export function startNativeNetworkProxy(hosts: readonly string[]): Promise<NativeNetworkProxy> {
	return Effect.runPromise(Effect.gen(function* () {
		const scope = yield* Scope.make();
		const acquire = acquireNativeNetworkProxy(hosts).pipe(Scope.provide(scope));
		const proxy = yield* acquire.pipe(Effect.onExit((exit) => Exit.isFailure(exit) ? Scope.close(scope, exit) : Effect.void));
		proxy.setOwnerScope(scope);
		return proxy;
	}));
}

const handleAccepted = Effect.fn("NativeNetworkProxy.handleAccepted")(function* (
	socket: Socket,
	allowed: ReadonlySet<string>,
	sockets: Set<Socket>,
) {
	yield* Effect.acquireRelease(
		Effect.sync(() => {
			const onClose = () => sockets.delete(socket);
			const onError = () => socket.destroy();
			socket.once("close", onClose);
			socket.on("error", onError);
			return { onClose, onError };
		}),
		({ onClose, onError }) => Effect.sync(() => {
			socket.removeListener("close", onClose);
			socket.removeListener("error", onError);
			sockets.delete(socket);
			socket.destroy();
		}),
	);
	const initial = yield* readAtLeast(socket, 1).pipe(Effect.timeout(HANDSHAKE_TIMEOUT_MS));
	if (initial[0] === 0x05) yield* handleSocks5(socket, initial, allowed, sockets);
	else yield* handleHttp(socket, initial, allowed, sockets);
});

const handleHttp = Effect.fn("NativeNetworkProxy.handleHttp")(function* (
	client: Socket,
	initial: Buffer,
	allowed: ReadonlySet<string>,
	sockets: Set<Socket>,
) {
	const request = yield* readThrough(client, initial, Buffer.from("\r\n\r\n"));
	const headerEnd = request.indexOf("\r\n\r\n");
	const head = request.subarray(0, headerEnd + 4).toString("latin1");
	const firstLine = head.slice(0, head.indexOf("\r\n"));
	const [method, target, version, ...extra] = firstLine.split(" ");
	if (!method || !target || !version || extra.length > 0 || !version.startsWith("HTTP/")) return yield* rejectHttp(client, 400, "Bad Request");

	if (method.toUpperCase() === "CONNECT") {
		const authority = parseAuthority(target, 443);
		if (!authority) return yield* rejectHttp(client, 400, "Bad CONNECT target");
		if (!allowed.has(authority.host)) return yield* rejectHttp(client, 403, "Host not approved");
		const upstream = yield* connectUpstream(authority.host, authority.port, sockets).pipe(
			Effect.catch(() => rejectHttp(client, 502, "Upstream connection failed").pipe(Effect.as(undefined))),
		);
		if (!upstream) return;
		client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
		const trailing = request.subarray(headerEnd + 4);
		if (trailing.length > 0) upstream.write(trailing);
		yield* pipeBoth(client, upstream);
		return;
	}

	const url = yield* Effect.try({ try: () => new URL(target), catch: proxyError }).pipe(
		Effect.catch(() => rejectHttp(client, 400, "Proxy requests need an absolute URL").pipe(Effect.as(undefined))),
	);
	if (!url) return;
	if (url.protocol !== "http:") return yield* rejectHttp(client, 400, "Use CONNECT for TLS");
	const host = yield* Effect.try({ try: () => normalizeNetworkHost(url.hostname), catch: proxyError });
	if (!allowed.has(host)) return yield* rejectHttp(client, 403, "Host not approved");
	const port = url.port ? Number(url.port) : 80;
	if (!validPort(port)) return yield* rejectHttp(client, 400, "Bad target port");
	const upstream = yield* connectUpstream(host, port, sockets).pipe(
		Effect.catch(() => rejectHttp(client, 502, "Upstream connection failed").pipe(Effect.as(undefined))),
	);
	if (!upstream) return;
	const path = `${url.pathname || "/"}${url.search}`;
	upstream.write(Buffer.from(head.replace(firstLine, `${method} ${path} ${version}`), "latin1"));
	const trailing = request.subarray(headerEnd + 4);
	if (trailing.length > 0) upstream.write(trailing);
	yield* pipeBoth(client, upstream);
});

const handleSocks5 = Effect.fn("NativeNetworkProxy.handleSocks5")(function* (
	client: Socket,
	initial: Buffer,
	allowed: ReadonlySet<string>,
	sockets: Set<Socket>,
) {
	let data = yield* readSocksBytes(client, initial, 2);
	const methods = data[1] ?? 0;
	data = yield* readSocksBytes(client, data, 2 + methods);
	if (!data.subarray(2, 2 + methods).includes(0x00)) { yield* endSocket(client, Buffer.from([0x05, 0xff])); return; }
	client.write(Buffer.from([0x05, 0x00]));
	data = yield* readSocksBytes(client, data.subarray(2 + methods), 4);
	if (data[0] !== 0x05 || data[1] !== 0x01 || data[2] !== 0x00) return yield* rejectSocks(client, 0x07);
	const type = data[3];
	let needed = type === 0x01 ? 10 : type === 0x04 ? 22 : 5;
	data = yield* readSocksBytes(client, data, needed);
	if (type === 0x03) { needed = 5 + (data[4] ?? 0) + 2; data = yield* readSocksBytes(client, data, needed); }
	const target = parseSocksTarget(data);
	if (!target) return yield* rejectSocks(client, 0x08);
	if (!allowed.has(target.host)) return yield* rejectSocks(client, 0x02);
	const upstream = yield* connectUpstream(target.host, target.port, sockets).pipe(
		Effect.catch(() => rejectSocks(client, 0x05).pipe(Effect.as(undefined))),
	);
	if (!upstream) return;
	client.write(Buffer.from([0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]));
	const trailing = data.subarray(needed);
	if (trailing.length > 0) upstream.write(trailing);
	yield* pipeBoth(client, upstream);
});

function parseSocksTarget(data: Buffer): { host: string; port: number } | undefined {
	const type = data[3]; let host: string; let offset: number;
	if (type === 0x01 && data.length >= 10) { host = `${data[4]}.${data[5]}.${data[6]}.${data[7]}`; offset = 8; }
	else if (type === 0x03 && data.length >= 7 + (data[4] ?? 0)) { const length = data[4] ?? 0; host = data.subarray(5, 5 + length).toString("ascii"); offset = 5 + length; }
	else if (type === 0x04 && data.length >= 22) { const groups: string[] = []; for (let index = 4; index < 20; index += 2) groups.push(data.readUInt16BE(index).toString(16)); host = groups.join(":"); offset = 20; }
	else return undefined;
	const port = data.readUInt16BE(offset); if (!validPort(port)) return undefined;
	try { return { host: normalizeNetworkHost(host), port }; } catch { return undefined; }
}

function parseAuthority(value: string, defaultPort: number): { host: string; port: number } | undefined {
	try { const url = new URL(`tcp://${value}`); const port = url.port ? Number(url.port) : defaultPort; if (!validPort(port)) return undefined; return { host: normalizeNetworkHost(url.hostname), port }; }
	catch { return undefined; }
}
function validPort(port: number): boolean { return Number.isInteger(port) && port >= 1 && port <= 65_535; }

const connectUpstream = Effect.fn("NativeNetworkProxy.connectUpstream")(function* (host: string, port: number, sockets: Set<Socket>) {
	if (sockets.size >= MAX_OPEN_SOCKETS) return yield* Effect.fail(proxyError("Native proxy connection limit reached"));
	const socket = yield* Effect.acquireRelease(
		Effect.sync(() => {
			const value = createConnection({ host, port });
			const onClose = () => sockets.delete(value);
			const onError = () => value.destroy();
			sockets.add(value);
			value.once("close", onClose);
			value.on("error", onError);
			return { value, onClose, onError };
		}),
		({ value, onClose, onError }) => Effect.sync(() => {
			value.removeListener("close", onClose);
			value.removeListener("error", onError);
			sockets.delete(value);
			value.destroy();
		}),
	).pipe(Effect.map(({ value }) => value));
	yield* Effect.callback<void, NativeNetworkProxyError>((resume) => {
		const onConnect = () => { cleanup(); resume(Effect.void); };
		const onError = (error: Error) => { cleanup(); resume(Effect.fail(proxyError(error))); };
		const cleanup = () => { socket.removeListener("connect", onConnect); socket.removeListener("error", onError); };
		socket.once("connect", onConnect); socket.once("error", onError);
		return Effect.sync(cleanup);
	}).pipe(Effect.timeout(CONNECT_TIMEOUT_MS));
	return socket;
});

const pipeBoth = Effect.fn("NativeNetworkProxy.pipeBoth")(function* (left: Socket, right: Socket) {
	let lastActivity = Date.now();
	const active = () => { lastActivity = Date.now(); };
	left.on("data", active); right.on("data", active);
	yield* Effect.addFinalizer(() => Effect.sync(() => { left.removeListener("data", active); right.removeListener("data", active); left.unpipe(right); right.unpipe(left); right.destroy(); left.destroy(); }));
	left.pipe(right); right.pipe(left);
	const monitor = Effect.forever(Effect.sleep(IDLE_TIMEOUT_MS).pipe(Effect.andThen(Effect.sync(() => {
		if (Date.now() - lastActivity >= IDLE_TIMEOUT_MS) { left.destroy(); right.destroy(); }
	}))));
	yield* Effect.forkScoped(monitor);
	yield* waitForSocketClose(left);
});

function rejectHttp(socket: Socket, status: number, message: string): Effect.Effect<void, NativeNetworkProxyError> {
	const body = `${message}\n`;
	return endSocket(socket, Buffer.from(`HTTP/1.1 ${status} ${message}\r\nConnection: close\r\nContent-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`));
}
function rejectSocks(socket: Socket, code: number): Effect.Effect<void, NativeNetworkProxyError> { return endSocket(socket, Buffer.from([0x05, code, 0x00, 0x01, 0, 0, 0, 0, 0, 0])); }
function endSocket(socket: Socket, data: Buffer): Effect.Effect<void, NativeNetworkProxyError> {
	if (socket.destroyed) return Effect.void;
	return Effect.callback<void, NativeNetworkProxyError>((resume) => {
		const cleanup = () => { socket.removeListener("close", onClose); socket.removeListener("error", onError); };
		const onClose = () => { cleanup(); resume(Effect.void); };
		const onError = (error: Error) => { cleanup(); resume(Effect.fail(proxyError(error))); };
		socket.once("close", onClose);
		socket.once("error", onError);
		socket.end(data);
		return Effect.sync(cleanup);
	});
}

const readThrough = Effect.fn("NativeNetworkProxy.readThrough")(function* (socket: Socket, initial: Buffer, delimiter: Buffer) {
	let data = initial;
	while (data.indexOf(delimiter) < 0) { if (data.length >= MAX_HANDSHAKE_BYTES) return yield* Effect.fail(proxyError("Proxy request headers are too large")); data = Buffer.concat([data, yield* nextChunk(socket)]); }
	return data;
});
const readSocksBytes = Effect.fn("NativeNetworkProxy.readSocksBytes")(function* (socket: Socket, initial: Buffer, count: number) {
	let data = initial;
	while (data.length < count) { if (data.length >= MAX_HANDSHAKE_BYTES) return yield* Effect.fail(proxyError("SOCKS request is too large")); data = Buffer.concat([data, yield* nextChunk(socket)]); }
	return data;
});
function readAtLeast(socket: Socket, count: number) { return readSocksBytes(socket, Buffer.alloc(0), count); }

function nextChunk(socket: Socket): Effect.Effect<Buffer, NativeNetworkProxyError> {
	return Effect.callback<Buffer, NativeNetworkProxyError>((resume) => {
		const cleanup = () => { socket.removeListener("data", onData); socket.removeListener("end", onEnd); socket.removeListener("error", onError); };
		const onData = (chunk: Buffer) => { cleanup(); socket.pause(); resume(Effect.succeed(chunk)); };
		const onEnd = () => { cleanup(); resume(Effect.fail(proxyError("Proxy client closed during handshake"))); };
		const onError = (error: Error) => { cleanup(); resume(Effect.fail(proxyError(error))); };
		socket.once("data", onData); socket.once("end", onEnd); socket.once("error", onError); socket.resume();
		return Effect.sync(cleanup);
	});
}

function listen(server: Server, options: { host: string; port: number } | { path: string }): Effect.Effect<void, NativeNetworkProxyError> {
	return Effect.callback<void, NativeNetworkProxyError>((resume) => {
		const onError = (error: Error) => { cleanup(); resume(Effect.fail(proxyError(error))); };
		const onListening = () => { cleanup(); resume(Effect.void); };
		const cleanup = () => { server.removeListener("error", onError); server.removeListener("listening", onListening); };
		server.once("error", onError); server.once("listening", onListening); server.listen(options);
		return Effect.sync(cleanup);
	});
}
function closeServer(server: Server): Effect.Effect<void> {
	if (!server.listening) return Effect.void;
	return Effect.callback<void>((resume) => {
		const done = () => resume(Effect.void);
		server.close(done); server.unref();
		return Effect.sync(() => server.removeListener("close", done));
	});
}
function waitForSocketClose(socket: Socket): Effect.Effect<void, NativeNetworkProxyError> {
	if (socket.destroyed) return Effect.void;
	return Effect.callback<void, NativeNetworkProxyError>((resume) => {
		const cleanup = () => { socket.removeListener("close", onClose); socket.removeListener("error", onError); };
		const onClose = () => { cleanup(); resume(Effect.void); };
		const onError = (error: Error) => { cleanup(); resume(Effect.fail(proxyError(error))); };
		socket.once("close", onClose); socket.once("error", onError);
		return Effect.sync(cleanup);
	});
}
