use std::io::{self, BufRead, IsTerminal, Write};

use zenv::{Error, Result};

pub(crate) fn confirm_recreation() -> Result<bool> {
    let input = io::stdin();
    confirm(
        input.is_terminal(),
        &mut input.lock(),
        &mut io::stderr().lock(),
    )
}

fn confirm(interactive: bool, input: &mut impl BufRead, output: &mut impl Write) -> Result<bool> {
    writeln!(
        output,
        "Warning: container creation settings do not match Zenvfile."
    )
    .map_err(|error| Error::io("stderr", error))?;
    if !interactive {
        return Err(Error::Config(
            "Cannot confirm recreation without interactive stdin. \
             Set run.check_zenvfile_changes = false for noninteractive use."
                .into(),
        ));
    }
    loop {
        write!(
            output,
            "Recreate the container with the updated settings? [y/N]: "
        )
        .and_then(|()| output.flush())
        .map_err(|error| Error::io("stderr", error))?;
        let mut answer = String::new();
        if input
            .read_line(&mut answer)
            .map_err(|error| Error::io("stdin", error))?
            == 0
        {
            return Err(Error::Config("Aborted.".into()));
        }
        match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "" | "n" | "no" => return Ok(false),
            _ => writeln!(output, "Please respond with yes or no.")
                .map_err(|error| Error::io("stderr", error))?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn noninteractive_confirmation_does_not_consume_stdin() {
        let mut input = Cursor::new(b"command payload\n");
        let mut output = Vec::new();
        let error = confirm(false, &mut input, &mut output).unwrap_err();
        assert!(error.to_string().contains("check_zenvfile_changes = false"));
        assert_eq!(input.position(), 0);
        assert!(String::from_utf8(output).unwrap().starts_with("Warning:"));
    }

    #[test]
    fn confirmation_defaults_to_no_and_accepts_explicit_answers() {
        for (answer, expected) in [
            ("\n", false),
            ("n\n", false),
            ("NO\n", false),
            ("y\n", true),
            ("Yes\n", true),
        ] {
            let mut output = Vec::new();
            assert_eq!(
                confirm(true, &mut Cursor::new(answer), &mut output).unwrap(),
                expected
            );
            assert!(String::from_utf8(output).unwrap().contains("[y/N]"));
        }
    }

    #[test]
    fn invalid_answer_reprompts_and_eof_aborts() {
        let mut output = Vec::new();
        assert!(confirm(true, &mut Cursor::new("invalid\ny\n"), &mut output).unwrap());
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("Please respond")
        );
        assert!(confirm(true, &mut Cursor::new(""), &mut Vec::new()).is_err());
    }
}
