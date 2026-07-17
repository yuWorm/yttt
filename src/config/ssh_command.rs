use std::path::{Path, PathBuf};

pub const DEFAULT_SSH_PORT: u16 = 22;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedSshCommand {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub identity_file: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SshCommandParseError {
    #[error("SSH command is empty")]
    Empty,
    #[error("expected an ssh command, found `{0}`")]
    NotSsh(String),
    #[error("invalid shell quoting: {0}")]
    InvalidQuoting(String),
    #[error("option `{0}` requires a value")]
    MissingOptionValue(String),
    #[error("invalid SSH port `{0}`")]
    InvalidPort(String),
    #[error("unsupported SSH option `{0}`")]
    UnsupportedOption(String),
    #[error("SSH destination is missing")]
    MissingDestination,
    #[error("invalid SSH destination `{0}`")]
    InvalidDestination(String),
    #[error("remote commands are not supported in this field")]
    RemoteCommandUnsupported,
}

pub fn parse_ssh_command(command: &str) -> Result<ParsedSshCommand, SshCommandParseError> {
    let words = shell_words::split(command)
        .map_err(|error| SshCommandParseError::InvalidQuoting(error.to_string()))?;
    let Some(program) = words.first() else {
        return Err(SshCommandParseError::Empty);
    };
    let program_name = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    if !matches!(program_name, "ssh" | "ssh.exe") {
        return Err(SshCommandParseError::NotSsh(program.clone()));
    }

    let mut port = DEFAULT_SSH_PORT;
    let mut user = None;
    let mut identity_file = None;
    let mut destination = None;
    let mut index = 1;
    while index < words.len() {
        let word = &words[index];
        if destination.is_some() {
            return Err(SshCommandParseError::RemoteCommandUnsupported);
        }
        if word == "--" {
            index += 1;
            let Some(value) = words.get(index) else {
                return Err(SshCommandParseError::MissingDestination);
            };
            destination = Some(value.clone());
            index += 1;
            continue;
        }
        if !word.starts_with('-') || word == "-" {
            destination = Some(word.clone());
            index += 1;
            continue;
        }

        match option_with_inline_value(word, "-p") {
            Some(Some(value)) => port = parse_port(value)?,
            Some(None) => {
                let value = next_option_value(&words, &mut index, "-p")?;
                port = parse_port(value)?;
            }
            None => match option_with_inline_value(word, "-l") {
                Some(Some(value)) => user = Some(non_empty_option(value, "-l")?.to_string()),
                Some(None) => user = Some(next_option_value(&words, &mut index, "-l")?.to_string()),
                None => match option_with_inline_value(word, "-i") {
                    Some(Some(value)) => {
                        identity_file = Some(PathBuf::from(non_empty_option(value, "-i")?))
                    }
                    Some(None) => {
                        identity_file =
                            Some(PathBuf::from(next_option_value(&words, &mut index, "-i")?))
                    }
                    None if word == "-o" => {
                        let option = next_option_value(&words, &mut index, "-o")?;
                        apply_config_option(option, &mut port, &mut user, &mut identity_file)?;
                    }
                    None if word.starts_with("-o") && word.len() > 2 => {
                        apply_config_option(&word[2..], &mut port, &mut user, &mut identity_file)?;
                    }
                    None if is_ignorable_flag(word) => {}
                    None => return Err(SshCommandParseError::UnsupportedOption(word.clone())),
                },
            },
        }
        index += 1;
    }

    let destination = destination.ok_or(SshCommandParseError::MissingDestination)?;
    let (destination_user, host, destination_port) = parse_destination(&destination)?;
    if let Some(destination_user) = destination_user {
        user = Some(destination_user);
    }
    if let Some(destination_port) = destination_port {
        port = destination_port;
    }

    Ok(ParsedSshCommand {
        host,
        port,
        user,
        identity_file,
    })
}

pub fn format_ssh_command(
    host: &str,
    port: u16,
    user: &str,
    identity_file: Option<&Path>,
) -> String {
    if host.trim().is_empty() {
        return "ssh ".to_string();
    }
    let mut words = vec!["ssh".to_string()];
    if port != DEFAULT_SSH_PORT {
        words.extend(["-p".to_string(), port.to_string()]);
    }
    if let Some(identity_file) = identity_file {
        words.extend([
            "-i".to_string(),
            identity_file.to_string_lossy().into_owned(),
        ]);
    }
    let destination = if user.trim().is_empty() {
        host.to_string()
    } else {
        format!("{}@{}", user.trim(), host)
    };
    words.push(destination);
    shell_words::join(words)
}

fn option_with_inline_value<'a>(word: &'a str, option: &str) -> Option<Option<&'a str>> {
    if word == option {
        Some(None)
    } else {
        word.strip_prefix(option).map(Some)
    }
}

fn next_option_value<'a>(
    words: &'a [String],
    index: &mut usize,
    option: &str,
) -> Result<&'a str, SshCommandParseError> {
    *index += 1;
    words
        .get(*index)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SshCommandParseError::MissingOptionValue(option.to_string()))
}

fn non_empty_option<'a>(value: &'a str, option: &str) -> Result<&'a str, SshCommandParseError> {
    if value.is_empty() {
        Err(SshCommandParseError::MissingOptionValue(option.to_string()))
    } else {
        Ok(value)
    }
}

fn parse_port(value: &str) -> Result<u16, SshCommandParseError> {
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| SshCommandParseError::InvalidPort(value.to_string()))
}

fn apply_config_option(
    option: &str,
    port: &mut u16,
    user: &mut Option<String>,
    identity_file: &mut Option<PathBuf>,
) -> Result<(), SshCommandParseError> {
    let (name, value) = option
        .split_once('=')
        .ok_or_else(|| SshCommandParseError::UnsupportedOption(format!("-o {option}")))?;
    match name.to_ascii_lowercase().as_str() {
        "port" => *port = parse_port(value)?,
        "user" if !value.is_empty() => *user = Some(value.to_string()),
        "identityfile" if !value.is_empty() => *identity_file = Some(PathBuf::from(value)),
        _ => {
            return Err(SshCommandParseError::UnsupportedOption(format!(
                "-o {name}"
            )));
        }
    }
    Ok(())
}

fn is_ignorable_flag(option: &str) -> bool {
    matches!(
        option,
        "-4" | "-6"
            | "-A"
            | "-a"
            | "-C"
            | "-K"
            | "-k"
            | "-N"
            | "-n"
            | "-q"
            | "-T"
            | "-t"
            | "-v"
            | "-X"
            | "-x"
            | "-Y"
            | "-y"
    )
}

fn parse_destination(
    destination: &str,
) -> Result<(Option<String>, String, Option<u16>), SshCommandParseError> {
    let destination = destination.strip_prefix("ssh://").unwrap_or(destination);
    let authority = destination.strip_suffix('/').unwrap_or(destination);
    if authority.is_empty() || authority.contains('/') {
        return Err(SshCommandParseError::InvalidDestination(
            destination.to_string(),
        ));
    }
    let (user, host_and_port) = match authority.rsplit_once('@') {
        Some((user, host)) if !user.is_empty() => (Some(user.to_string()), host),
        Some(_) => {
            return Err(SshCommandParseError::InvalidDestination(
                destination.to_string(),
            ));
        }
        None => (None, authority),
    };
    let (host, port) = parse_host_and_port(host_and_port)
        .ok_or_else(|| SshCommandParseError::InvalidDestination(destination.to_string()))?;
    Ok((user, host, port))
}

fn parse_host_and_port(value: &str) -> Option<(String, Option<u16>)> {
    if let Some(rest) = value.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']')?;
        if host.is_empty() {
            return None;
        }
        let port = if suffix.is_empty() {
            None
        } else {
            Some(parse_port(suffix.strip_prefix(':')?).ok()?)
        };
        return Some((host.to_string(), port));
    }
    if value.is_empty() {
        return None;
    }
    if let Some((host, port)) = value.rsplit_once(':')
        && !host.contains(':')
    {
        return Some((host.to_string(), Some(parse_port(port).ok()?)));
    }
    Some((value.to_string(), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_ssh_command_and_quoted_identity() {
        assert_eq!(
            parse_ssh_command("ssh -p 2222 -i '/Users/me/SSH Keys/dev' alice@example.com").unwrap(),
            ParsedSshCommand {
                host: "example.com".to_string(),
                port: 2222,
                user: Some("alice".to_string()),
                identity_file: Some(PathBuf::from("/Users/me/SSH Keys/dev")),
            }
        );
    }

    #[test]
    fn parses_uri_and_openssh_style_options() {
        assert_eq!(
            parse_ssh_command(
                "ssh -o User=alice -oPort=2200 -oIdentityFile=~/.ssh/id_ed25519 ssh://example.com"
            )
            .unwrap(),
            ParsedSshCommand {
                host: "example.com".to_string(),
                port: 2200,
                user: Some("alice".to_string()),
                identity_file: Some(PathBuf::from("~/.ssh/id_ed25519")),
            }
        );
    }

    #[test]
    fn destination_overrides_user_and_port_options() {
        let parsed = parse_ssh_command("ssh -l old -p 22 new@[::1]:2222").unwrap();
        assert_eq!(parsed.host, "::1");
        assert_eq!(parsed.user.as_deref(), Some("new"));
        assert_eq!(parsed.port, 2222);
    }

    #[test]
    fn rejects_remote_commands_and_unsupported_connection_options() {
        assert_eq!(
            parse_ssh_command("ssh alice@example.com uptime"),
            Err(SshCommandParseError::RemoteCommandUnsupported)
        );
        assert_eq!(
            parse_ssh_command("ssh -J bastion alice@example.com"),
            Err(SshCommandParseError::UnsupportedOption("-J".to_string()))
        );
    }

    #[test]
    fn formats_a_reparseable_command() {
        let command = format_ssh_command(
            "example.com",
            2222,
            "alice",
            Some(Path::new("/Users/me/SSH Keys/dev")),
        );
        assert_eq!(
            parse_ssh_command(&command).unwrap(),
            ParsedSshCommand {
                host: "example.com".to_string(),
                port: 2222,
                user: Some("alice".to_string()),
                identity_file: Some(PathBuf::from("/Users/me/SSH Keys/dev")),
            }
        );
    }
}
