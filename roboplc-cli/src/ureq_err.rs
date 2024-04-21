use colored::Colorize as _;

pub trait PrintErr<T> {
    fn process_error(self) -> Result<T, Box<dyn std::error::Error>>;
}

impl<T> PrintErr<T> for Result<T, ureq::Error> {
    fn process_error(self) -> Result<T, Box<dyn std::error::Error>> {
        match self {
            Ok(v) => Ok(v),
            Err(e) => match e.kind() {
                ureq::ErrorKind::HTTP => {
                    let response = e.into_response().unwrap();
                    let status = response.status();
                    let msg = format!(
                        "{} ({})",
                        response.into_string().unwrap_or_default(),
                        status
                    );
                    eprintln!("{}: {}", "Error".red(), msg);
                    Err("Remote".into())
                }
                _ => Err(e.into()),
            },
        }
    }
}
