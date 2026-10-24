//! Parser inputs below are inert TypeScript data. No fixture is executed.
use std::fs;

use anyhow::Result;
use harvest_omp::cleanup_scan::{Options, scan};
use harvest_runtime::cancel::CancelToken;

fn fixture() -> Result<tempfile::TempDir> {
	let root = tempfile::tempdir()?;
	let source = root.path().join("packages/demo/src");
	fs::create_dir_all(&source)?;
	fs::create_dir(root.path().join("scripts"))?;
	fs::write(
		root.path().join("packages/demo/package.json"),
		r#"{"exports":{".":"./src/index.ts","./*":"./src/*.ts"}}"#,
	)?;
	fs::write(
		source.join("index.ts"),
		"export * from './public';\nexport { aliasValue as publishedAlias } from './hidden';\n",
	)?;
	fs::write(
		source.join("public.ts"),
		"export const GhostOnlyTests = 1;\nexport interface PublicShape { value: string }\nexport \
		 default class NamedDefault { constructor() {} get value() { return 1 } set value(v: \
		 number) {} method() {} }\n",
	)?;
	fs::write(
		source.join("hidden.ts"),
		"export const OnlyHiddenNobodyUses = 2;\nexport const aliasValue = 3;\n",
	)?;
	fs::write(source.join("public.test.ts"), "const checks = GhostOnlyTests;\n")?;
	fs::write(root.path().join("scripts/probe.ts"), "const otherConsumer = PublicShape;\n")?;
	let mut large = "import { something } from '../../../outside';\nexport class LargeOwner \
	                 {\nconstructor() {}\n"
		.to_owned();
	for i in 0..12 {
		large.push_str(&format!("method{i}() {{ return {i}; }}\n"));
	}
	large.push_str(
		"}\nconst suspicious = input as unknown;\nconst optional = source?.value ?? fallback;\nif \
		 (typeof source !== 'undefined') Array.isArray(source);\n",
	);
	for i in 0..805 {
		large.push_str(&format!("// retained line {i}\n"));
	}
	fs::write(source.join("utils.ts"), large)?;
	for (file, number) in [("clone-a", 4), ("clone-b", 7)] {
		let text = (0..16)
			.map(|i| format!("const item{i} = source.read({number});\n"))
			.collect::<String>();
		fs::write(source.join(format!("{file}.ts")), text)?;
	}
	fs::write(source.join("misc-helpers.ts"), "export const JunkEntryNobodyUses = 1;\n")?;
	Ok(root)
}
fn options() -> Options {
	Options { packages: Some(vec!["demo".into()]), ..Default::default() }
}
#[test]
fn all_six_reports_preserve_the_legacy_fixture_contract() -> Result<()> {
	let root = fixture()?;
	let report = scan(root.path(), &options(), &Default::default())?;
	assert!(report.diagnostics.is_empty());
	assert_eq!(report.scanned_files, 7);
	assert_eq!(report.corpus_files, 9);
	let god = &report.god_objects[0];
	assert_eq!((god.loc, god.exports, god.top_level_statements, god.god_score), (824, 1, 5, 1195));
	assert_eq!((god.classes[0].name.as_str(), god.classes[0].methods), ("LargeOwner", 12));
	assert_eq!(report.clones[0].duplicated_sig_lines, 32);
	assert_eq!(report.clones[0].regions, vec![
		"packages/demo/src/clone-a.ts:1-16",
		"packages/demo/src/clone-b.ts:1-16"
	]);
	assert_eq!(report.junk_drawers[0].file, "packages/demo/src/misc-helpers.ts");
	let protected = report
		.dead_exports
		.iter()
		.find(|e| e.name == "GhostOnlyTests")
		.expect("protected candidate");
	assert_eq!(protected.tier, "barrel-public");
	assert!(protected.test_only);
	assert!(
		!report
			.dead_exports
			.iter()
			.any(|e| e.name == "PublicShape" || e.name == "aliasValue")
	);
	let hidden = report
		.dead_exports
		.iter()
		.find(|e| e.name == "OnlyHiddenNobodyUses")
		.expect("hidden candidate");
	assert_eq!(hidden.tier, "wildcard-only");
	assert!(!hidden.test_only);
	assert_eq!(report.deep_imports[0].count, 1);
	let checks = &report.check_density[0];
	assert_eq!(checks.per100, 0.6);
	assert_eq!(
		(
			checks.checks.as_casts,
			checks.checks.opt_chain,
			checks.checks.coalesce,
			checks.checks.typeof_narrow,
			checks.checks.is_array
		),
		(1, 1, 1, 1, 1)
	);
	let mut output = Vec::new();
	report.write_text(&mut output)?;
	assert!(String::from_utf8(output)?.contains("PROTECTED, do not delete"));
	Ok(())
}
#[test]
fn declarations_aliases_escapes_and_class_methods_are_structural() -> Result<()> {
	let code = r#"// inert structural fixture
export declare const FirstValue: string, OtherValue: number;
export default function NamedDefaultFunction() {}
export class ExampleOwner { constructor() {} get value() { return 1 } set value(v: number) {} ordinary() {} }
export type AliasType = string;
export namespace PublicSpace { export const inner = 1 }
export { FirstValue as RenamedValue };
export { remote as exposed } from './\u0062ase';
export * from './\x6fther';
export * as namespaceOnly from './namespace';
const ignored = { export: 'fake' };
"#;
	let facts = pi_ast::cleanup::inspect(code, &mut || Ok(()))?;
	assert!(!facts.recovered);
	for name in [
		"FirstValue",
		"OtherValue",
		"NamedDefaultFunction",
		"ExampleOwner",
		"AliasType",
		"PublicSpace",
		"RenamedValue",
	] {
		assert!(facts.exported_names.iter().any(|value| value == name), "missing {name}");
	}
	assert!(
		!facts
			.exported_names
			.iter()
			.any(|name| name == "inner" || name == "ignored")
	);
	assert_eq!(facts.classes[0].methods, 3);
	assert_eq!(facts.named_reexports[0].name, "exposed");
	assert_eq!(facts.named_reexports[0].from, "./base");
	assert_eq!(facts.export_star, vec!["./other"]);
	Ok(())
}
#[test]
fn invalid_selection_and_parent_cancellation_fail_without_writes() -> Result<()> {
	let root = fixture()?;
	let mut invalid = options();
	invalid.packages = Some(vec!["../outside".into()]);
	assert!(scan(root.path(), &invalid, &Default::default()).is_err());
	let mut invalid = options();
	invalid.top = 0;
	assert!(scan(root.path(), &invalid, &Default::default()).is_err());
	let token = CancelToken::with_timeout(Some(std::time::Duration::ZERO));
	assert!(scan(root.path(), &options(), &token).is_err());
	assert!(!root.path().join("cleanup-report.json").exists());
	Ok(())
}
#[cfg(unix)]
#[test]
fn corpus_does_not_follow_external_symlinks_or_generated_directories() -> Result<()> {
	use std::os::unix::fs::symlink;
	let root = fixture()?;
	let outside = tempfile::tempdir()?;
	fs::write(outside.path().join("leak.ts"), "export const OutsideSymbolNeverRead = 1;")?;
	symlink(outside.path(), root.path().join("packages/demo/src/escape"))?;
	fs::create_dir(root.path().join("packages/demo/src/schema-gen"))?;
	fs::write(
		root.path().join("packages/demo/src/schema-gen/excluded.ts"),
		"export const GeneratedNeverRead = 1;",
	)?;
	let report = scan(root.path(), &options(), &Default::default())?;
	assert!(
		!report
			.dead_exports
			.iter()
			.any(|e| matches!(e.name.as_str(), "OutsideSymbolNeverRead" | "GeneratedNeverRead"))
	);
	assert_eq!(report.corpus_files, 9);
	Ok(())
}

#[test]
fn copied_native_command_operates_without_legacy_runtimes_in_path() -> Result<()> {
	let root = fixture()?;
	let installed = tempfile::tempdir()?;
	let executable = installed
		.path()
		.join(format!("harvest{}", std::env::consts::EXE_SUFFIX));
	fs::copy(env!("CARGO_BIN_EXE_harvest"), &executable)?;
	let output = std::process::Command::new(&executable)
		.args(["cleanup-scan", "--json", "--pkg=demo", "--workspace"])
		.arg(root.path())
		.env("PATH", installed.path().join("no-runtimes"))
		.output()?;
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
	assert_eq!(report["godObjects"][0]["classes"][0]["methods"], 12);
	assert_eq!(report["clones"][0]["duplicatedSigLines"], 32);
	assert_eq!(report["deepImports"][0]["count"], 1);
	let invalid = std::process::Command::new(&executable)
		.args(["cleanup-scan", "--top=0"])
		.env("PATH", installed.path().join("no-runtimes"))
		.output()?;
	assert!(!invalid.status.success());
	Ok(())
}
