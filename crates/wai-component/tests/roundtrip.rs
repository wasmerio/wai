use anyhow::{Context, Result};
use pretty_assertions::assert_eq;
use std::fs;
use wai_component::{decode_interface_component, InterfaceEncoder, InterfacePrinter};
use wai_parser::Interface;

/// Tests the the roundtrip encoding of individual interface files.
///
/// This test looks in the `interfaces/` directory for test cases.
///
/// Each test case is a directory containing a `<testcase>.wai` file.
///
/// The test encodes the wai file, decodes the resulting bytes, and
/// compares the generated interface definition to the original interface
/// definition.
///
/// Run the test with the environment variable `BLESS` set to update
/// the wai file based on the decoded output.
#[test]
fn roundtrip_interfaces() -> Result<()> {
    for entry in fs::read_dir("tests/interfaces")? {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }

        let test_case = path.file_stem().unwrap().to_str().unwrap();
        let wai_path = path.join(test_case).with_extension("wai");

        let interface = Interface::parse_file(&wai_path).context("failed to parse `wai` file")?;

        let encoder = InterfaceEncoder::new(&interface).validate(true);

        let bytes = encoder.encode().with_context(|| {
            format!(
                "failed to encode a component from interface `{}` for test case `{}`",
                path.display(),
                test_case,
            )
        })?;

        let interface = decode_interface_component(&bytes).context("failed to decode bytes")?;

        let mut printer = InterfacePrinter::default();
        let output = printer
            .print(&interface)
            .context("failed to print interface")?;

        if std::env::var_os("BLESS").is_some() {
            fs::write(&wai_path, output)?;
        } else {
            assert_eq!(
                fs::read_to_string(&wai_path)?.replace("\r\n", "\n"),
                output,
                "encoding of wai file `{}` did not match the the decoded interface for test case `{}`",
                wai_path.display(),
                test_case,
            );
        }
    }

    Ok(())
}
