import assert from "node:assert/strict";
import { createServer as createHttpServer } from "node:http";
import { connect } from "node:net";
import { stat } from "node:fs/promises";
import test from "node:test";
import { startNativeNetworkProxy } from "./native-network-proxy.ts";

test("native proxy forwards approved HTTP hosts and blocks other hosts", async () => {
	const upstream = createHttpServer((_request, response) => response.end("proxy-ok"));
	await new Promise<void>((resolve) => upstream.listen(0, "127.0.0.1", resolve));
	const address = upstream.address();
	assert(address && typeof address !== "string");
	const proxy = await startNativeNetworkProxy(["127.0.0.1"]);
	try {
		assert.equal((await stat(proxy.socketPath)).isSocket(), true);
		const allowed = await proxyRequest(
			proxy.port,
			`GET http://127.0.0.1:${address.port}/check HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n`,
		);
		assert.match(allowed, /proxy-ok/);
		const tunnel = await proxyRequest(
			proxy.port,
			`CONNECT 127.0.0.1:${address.port} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n` +
				"GET /tunnel HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
		);
		assert.match(tunnel, /^HTTP\/1\.1 200 Connection Established/);
		assert.match(tunnel, /proxy-ok/);
		const denied = await proxyRequest(
			proxy.port,
			`CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\n`,
		);
		assert.match(denied, /^HTTP\/1\.1 403/);
	} finally {
		await proxy.close();
		await new Promise<void>((resolve) => upstream.close(() => resolve()));
	}
});

test("native proxy accepts SOCKS5 only for approved hosts", async () => {
	const upstream = createHttpServer((_request, response) => response.end("socks-ok"));
	await new Promise<void>((resolve) => upstream.listen(0, "127.0.0.1", resolve));
	const address = upstream.address();
	assert(address && typeof address !== "string");
	const proxy = await startNativeNetworkProxy(["127.0.0.1"]);
	const socket = connect(proxy.port, "127.0.0.1");
	try {
		await onceConnected(socket);
		socket.write(Buffer.from([0x05, 0x01, 0x00]));
		assert.deepEqual(await readBytes(socket, 2), Buffer.from([0x05, 0x00]));
		const port = Buffer.alloc(2);
		port.writeUInt16BE(address.port);
		socket.write(Buffer.concat([Buffer.from([0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1]), port]));
		assert.equal((await readBytes(socket, 10))[1], 0x00);
		socket.write("GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
		const response = await readAll(socket);
		assert.match(response, /socks-ok/);
	} finally {
		socket.destroy();
		await proxy.close();
		await new Promise<void>((resolve) => upstream.close(() => resolve()));
	}
});

function proxyRequest(port: number, request: string): Promise<string> {
	const socket = connect(port, "127.0.0.1");
	return onceConnected(socket).then(() => {
		const response = readAll(socket);
		socket.write(request);
		return response;
	});
}

function onceConnected(socket: import("node:net").Socket): Promise<void> {
	return new Promise((resolve, reject) => {
		socket.once("connect", resolve);
		socket.once("error", reject);
	});
}

function readBytes(socket: import("node:net").Socket, count: number): Promise<Buffer> {
	return new Promise((resolve, reject) => {
		let data = Buffer.alloc(0);
		const onData = (chunk: Buffer) => {
			data = Buffer.concat([data, chunk]);
			if (data.length < count) return;
			cleanup();
			socket.pause();
			resolve(data.subarray(0, count));
		};
		const cleanup = () => {
			socket.removeListener("data", onData);
			socket.removeListener("error", onError);
		};
		const onError = (error: Error) => { cleanup(); reject(error); };
		socket.on("data", onData);
		socket.once("error", onError);
		socket.resume();
	});
}

function readAll(socket: import("node:net").Socket): Promise<string> {
	return new Promise((resolve, reject) => {
		const chunks: Buffer[] = [];
		socket.on("data", (chunk) => chunks.push(chunk));
		socket.once("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
		socket.once("error", reject);
		socket.resume();
	});
}
