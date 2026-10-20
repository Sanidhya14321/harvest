import { $ } from "bun";

await $`cargo xtask npm-versions ${Bun.argv.slice(2)}`;
