use rx_package::release;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() != 4 {
        return Err("usage: release_probe RELEASE REVOCATIONS INVENTORY PAYLOAD".into());
    }
    let payload = std::fs::read(&arguments[3])?;
    match release::verify(
        &std::fs::read(&arguments[0])?,
        &std::fs::read(&arguments[1])?,
        &std::fs::read(&arguments[2])?,
        |path, external| {
            if path != "tools/inert.txt" || external {
                return Err(release::Error::Content("probe path".into()));
            }
            Ok(payload.clone())
        },
    ) {
        Ok(value) => {
            println!("accepted version={}", value.version().0);
            Ok(())
        }
        Err(error) => {
            println!("refused condition={}", error.condition());
            std::process::exit(3)
        }
    }
}
