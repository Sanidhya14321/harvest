import * as path from "node:path";
import { generateProtoTs, ProtoContext, parseProto } from "./proto-parser";
import protocolRoots from "./protocol-roots.json" with { type: "json" };

const PACKAGES_DIR = path.resolve(import.meta.dir, "../..");
const CURSOR_PROTO = path.join(PACKAGES_DIR, "ai/src/providers/cursor/proto/agent.proto");
const DEVIN_PROTO_DIR = path.join(PACKAGES_DIR, "ai/src/providers/devin/proto");
const DISCOVERY_DIR = path.resolve(import.meta.dir, "../src/discovery");

// Explicit roots survive removal of TypeScript consumers during the Rust port.
// The protobuf compiler still resolves and validates all transitive dependencies.
async function parseProtoDirectory(directory: string): Promise<ProtoContext> {
	const context = new ProtoContext();
	const paths: string[] = [];
	const glob = new Bun.Glob("**/*.proto");
	for await (const relativePath of glob.scan({ cwd: directory, onlyFiles: true })) paths.push(relativePath);

	for (const relativePath of paths.sort()) {
		const source = await Bun.file(path.join(directory, relativePath)).text();
		context.addFile(parseProto(source, relativePath));
	}
	return context;
}

async function generateProtocols(): Promise<void> {
	if (protocolRoots.version !== 1) throw new Error("Unsupported protocol roots version");
	for (const roots of [protocolRoots.cursor, protocolRoots.devin]) {
		if (roots.messages.length === 0) throw new Error("Protocol message roots must not be empty");
		for (const symbols of [roots.messages, roots.enums]) {
			if (new Set(symbols).size !== symbols.length) throw new Error("Protocol roots contain duplicate symbols");
		}
	}
	const cursorSource = await Bun.file(CURSOR_PROTO).text();
	const cursorContext = new ProtoContext();
	cursorContext.addFile(parseProto(cursorSource, "agent.proto"));

	const cursor = generateProtoTs(cursorContext, {
		includeMessages: protocolRoots.cursor.messages,
		includeEnums: protocolRoots.cursor.enums,
		includeDependencies: true,
		packagePrefix: "Cursor agent",
		protobufImportPath: "./protobuf",
	});
	const devin = generateProtoTs(await parseProtoDirectory(DEVIN_PROTO_DIR), {
		includeMessages: protocolRoots.devin.messages,
		includeEnums: protocolRoots.devin.enums,
		includeDependencies: true,
		packagePrefix: "Devin",
		protobufImportPath: "./protobuf",
	});

	await Bun.write(path.join(DISCOVERY_DIR, "cursor-proto.ts"), `${cursor}\n`);
	await Bun.write(path.join(DISCOVERY_DIR, "devin-proto.ts"), `${devin}\n`);
}

await generateProtocols();
