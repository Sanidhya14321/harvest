import { isBunTestRuntime } from "@harvest/pi-utils/env";

process.stdout.write(JSON.stringify(isBunTestRuntime()));
