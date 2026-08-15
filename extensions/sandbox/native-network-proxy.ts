import { mkdtemp, rm } from "node:fs/promises";
import { createConnection, createServer, type Server, type Socket } from "node:net";
import { join } from "node:path";
import { normalizeNetworkHost } from "./io-permissions.ts";

const MAX_HANDSHAKE_BYTES = 64 * 1024;
const MAX_OPEN_SOCKETS = 512;
const HANDSHAKE_TIMEOUT_MS = 15_000;
const CONNECT_TIMEOUT_MS = 15_000;
const IDLE_TIMEOUT_MS = 5 * 60_000;

export interface NativeNetworkProxy {
	readonly port: number;
	readonly socketPath: string;
	close(): Promise<void>;
}

export async function startNativeNetworkProxy(
	hosts: readonly string[],
): Promise<NativeNetworkProxy> {
	const allowed = new Set(hosts.map(normalizeNetworkHost));
	if (allowed.size === 0) throw new Error("A native network proxy needs at least one host");

	// Unix socket paths are short on macOS. `/tmp` canonicalizes to
	// `/private/tmp` and keeps the generated path within that limit.
	const directory = await mkdtemp(join("/tmp", "pi-native-proxy-"));
	const socketPath = join(directory, "proxy.sock");
	const clients = new Set<Socket>();
	const accept = (socket: Socket) => {
		if (clients.size >= MAX_OPEN_SOCKETS) {
			socket.destroy();
			return;
		}
		clients.add(socket);
		socket.once("close", () => clients.delete(socket));
		acceptClient(socket, allowed, clients);
	};
	const tcp = createServer(accept);
	const unix = createServer(accept);
	try {
		await listen(tcp, { host: "127.0.0.1", port: 0 });
		await listen(unix, { path: socketPath });
	} catch (error) {
		await closeServer(tcp);
		await closeServer(unix);
		await rm(directory, { recursive: true, force: true });
		throw error;
	}
	const address = tcp.address();
	if (!address || typeof address === "string") throw new Error("Native proxy has no TCP port");
	let closed = false;
	return {
		port: address.port,
		socketPath,
		async close() {
			if (closed) return;
			closed = true;
			for (const client of clients) client.destroy();
			await Promise.all([closeServer(tcp), closeServer(unix)]);
			await rm(directory, { recursive: true, force: true });
		},
	};
}

function acceptClient(socket: Socket, allowed: ReadonlySet<string>, sockets: Set<Socket>): void {
	socket.setTimeout(HANDSHAKE_TIMEOUT_MS, () => socket.destroy());
	socket.once("error", () => socket.destroy());
	void readAtLeast(socket, 1)
		.then((initial) => {
			if (initial[0] === 0x05) return handleSocks5(socket, initial, allowed, sockets);
			return handleHttp(socket, initial, allowed, sockets);
		})
		.catch(() => socket.destroy());
}

async function handleHttp(
	client: Socket,
	initial: Buffer,
	allowed: ReadonlySet<string>,
	sockets: Set<Socket>,
): Promise<void> {
	const request = await readThrough(client, initial, Buffer.from("\r\n\r\n"));
	const headerEnd = request.indexOf("\r\n\r\n");
	const head = request.subarray(0, headerEnd + 4).toString("latin1");
	const firstLine = head.slice(0, head.indexOf("\r\n"));
	const [method, target, version, ...extra] = firstLine.split(" ");
	if (!method || !target || !version || extra.length > 0 || !version.startsWith("HTTP/")) {
		return rejectHttp(client, 400, "Bad Request");
	}

	if (method.toUpperCase() === "CONNECT") {
		const authority = parseAuthority(target, 443);
		if (!authority) return rejectHttp(client, 400, "Bad CONNECT target");
		if (!allowed.has(authority.host)) return rejectHttp(client, 403, "Host not approved");
		let upstream: Socket;
		try {
			upstream = await connectUpstream(authority.host, authority.port, sockets);
		} catch {
			return rejectHttp(client, 502, "Upstream connection failed");
		}
		client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
		const trailing = request.subarray(headerEnd + 4);
		if (trailing.length > 0) upstream.write(trailing);
		pipeBoth(client, upstream);
		return;
	}

	let url: URL;
	try {
		url = new URL(target);
	} catch {
		return rejectHttp(client, 400, "Proxy requests need an absolute URL");
	}
	if (url.protocol !== "http:") return rejectHttp(client, 400, "Use CONNECT for TLS");
	const host = normalizeNetworkHost(url.hostname);
	if (!allowed.has(host)) return rejectHttp(client, 403, "Host not approved");
	const port = url.port ? Number(url.port) : 80;
	if (!validPort(port)) return rejectHttp(client, 400, "Bad target port");
	let upstream: Socket;
	try {
		upstream = await connectUpstream(host, port, sockets);
	} catch {
		return rejectHttp(client, 502, "Upstream connection failed");
	}
	const path = `${url.pathname || "/"}${url.search}`;
	const rewritten = Buffer.from(head.replace(firstLine, `${method} ${path} ${version}`), "latin1");
	upstream.write(rewritten);
	const trailing = request.subarray(headerEnd + 4);
	if (trailing.length > 0) upstream.write(trailing);
	pipeBoth(client, upstream);
}

async function handleSocks5(
	client: Socket,
	initial: Buffer,
	allowed: ReadonlySet<string>,
	sockets: Set<Socket>,
): Promise<void> {
	let data = await readSocksBytes(client, initial, 2);
	const methods = data[1] ?? 0;
	data = await readSocksBytes(client, data, 2 + methods);
	if (!data.subarray(2, 2 + methods).includes(0x00)) {
		client.end(Buffer.from([0x05, 0xff]));
		return;
	}
	client.write(Buffer.from([0x05, 0x00]));
	data = await readSocksBytes(client, data.subarray(2 + methods), 4);
	if (data[0] !== 0x05 || data[1] !== 0x01 || data[2] !== 0x00) {
		return rejectSocks(client, 0x07);
	}
	const type = data[3];
	let needed = type === 0x01 ? 10 : type === 0x04 ? 22 : 5;
	data = await readSocksBytes(client, data, needed);
	if (type === 0x03) {
		needed = 5 + (data[4] ?? 0) + 2;
		data = await readSocksBytes(client, data, needed);
	}
	const target = parseSocksTarget(data);
	if (!target) return rejectSocks(client, 0x08);
	if (!allowed.has(target.host)) return rejectSocks(client, 0x02);
	let upstream: Socket;
	try {
		upstream = await connectUpstream(target.host, target.port, sockets);
	} catch {
		return rejectSocks(client, 0x05);
	}
	client.write(Buffer.from([0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]));
	const trailing = data.subarray(needed);
	if (trailing.length > 0) upstream.write(trailing);
	pipeBoth(client, upstream);
}

function parseSocksTarget(data: Buffer): { host: string; port: number } | undefined {
	const type = data[3];
	let host: string;
	let offset: number;
	if (type === 0x01 && data.length >= 10) {
		host = `${data[4]}.${data[5]}.${data[6]}.${data[7]}`;
		offset = 8;
	} else if (type === 0x03 && data.length >= 7 + (data[4] ?? 0)) {
		const length = data[4] ?? 0;
		host = data.subarray(5, 5 + length).toString("ascii");
		offset = 5 + length;
	} else if (type === 0x04 && data.length >= 22) {
		const groups: string[] = [];
		for (let index = 4; index < 20; index += 2) groups.push(data.readUInt16BE(index).toString(16));
		host = groups.join(":");
		offset = 20;
	} else return undefined;
	const port = data.readUInt16BE(offset);
	if (!validPort(port)) return undefined;
	try {
		return { host: normalizeNetworkHost(host), port };
	} catch {
		return undefined;
	}
}

function parseAuthority(value: string, defaultPort: number): { host: string; port: number } | undefined {
	try {
		const url = new URL(`tcp://${value}`);
		const port = url.port ? Number(url.port) : defaultPort;
		if (!validPort(port)) return undefined;
		return { host: normalizeNetworkHost(url.hostname), port };
	} catch {
		return undefined;
	}
}

function validPort(port: number): boolean {
	return Number.isInteger(port) && port >= 1 && port <= 65_535;
}

function connectUpstream(host: string, port: number, sockets: Set<Socket>): Promise<Socket> {
	return new Promise((resolve, reject) => {
		if (sockets.size >= MAX_OPEN_SOCKETS) {
			reject(new Error("Native proxy connection limit reached"));
			return;
		}
		const socket = createConnection({ host, port });
		sockets.add(socket);
		socket.once("close", () => sockets.delete(socket));
		const timeout = setTimeout(() => socket.destroy(new Error("Proxy connection timed out")), CONNECT_TIMEOUT_MS);
		const onError = (error: Error) => {
			clearTimeout(timeout);
			reject(error);
		};
		socket.once("error", onError);
		socket.once("connect", () => {
			clearTimeout(timeout);
			socket.removeListener("error", onError);
			socket.once("error", () => socket.destroy());
			resolve(socket);
		});
	});
}

function pipeBoth(left: Socket, right: Socket): void {
	left.setTimeout(IDLE_TIMEOUT_MS, () => left.destroy());
	right.setTimeout(IDLE_TIMEOUT_MS, () => right.destroy());
	left.pipe(right);
	right.pipe(left);
	left.once("close", () => right.destroy());
	right.once("close", () => left.destroy());
}

function rejectHttp(socket: Socket, status: number, message: string): void {
	const body = `${message}\n`;
	socket.end(
		`HTTP/1.1 ${status} ${message}\r\nConnection: close\r\nContent-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`,
	);
}

function rejectSocks(socket: Socket, code: number): void {
	socket.end(Buffer.from([0x05, code, 0x00, 0x01, 0, 0, 0, 0, 0, 0]));
}

async function readThrough(socket: Socket, initial: Buffer, delimiter: Buffer): Promise<Buffer> {
	let data = initial;
	while (data.indexOf(delimiter) < 0) {
		if (data.length >= MAX_HANDSHAKE_BYTES) throw new Error("Proxy request headers are too large");
		data = Buffer.concat([data, await nextChunk(socket)]);
	}
	return data;
}

async function readSocksBytes(socket: Socket, initial: Buffer, count: number): Promise<Buffer> {
	let data = initial;
	while (data.length < count) {
		if (data.length >= MAX_HANDSHAKE_BYTES) throw new Error("SOCKS request is too large");
		data = Buffer.concat([data, await nextChunk(socket)]);
	}
	return data;
}

async function readAtLeast(socket: Socket, count: number): Promise<Buffer> {
	return readSocksBytes(socket, Buffer.alloc(0), count);
}

function nextChunk(socket: Socket): Promise<Buffer> {
	return new Promise((resolve, reject) => {
		const cleanup = () => {
			socket.removeListener("data", onData);
			socket.removeListener("end", onEnd);
			socket.removeListener("error", onError);
		};
		const onData = (chunk: Buffer) => {
			cleanup();
			socket.pause();
			resolve(chunk);
		};
		const onEnd = () => {
			cleanup();
			reject(new Error("Proxy client closed during handshake"));
		};
		const onError = (error: Error) => {
			cleanup();
			reject(error);
		};
		socket.once("data", onData);
		socket.once("end", onEnd);
		socket.once("error", onError);
		socket.resume();
	});
}

function listen(server: Server, options: { host: string; port: number } | { path: string }): Promise<void> {
	return new Promise((resolve, reject) => {
		server.once("error", reject);
		server.listen(options, () => {
			server.removeListener("error", reject);
			resolve();
		});
	});
}

function closeServer(server: Server): Promise<void> {
	if (!server.listening) return Promise.resolve();
	return new Promise((resolve) => {
		server.close(() => resolve());
		server.unref();
	});
}
