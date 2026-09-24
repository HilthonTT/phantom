use phantom_core::{Err, Result, err};

const MAX_EMAIL_LEN: usize = 500;

pub fn canonicalize_email(address: &str) -> Result<String> {
    if address.len() > MAX_EMAIL_LEN {
        return Err!(Request(InvalidParam("Email address is too long")));
    }

    let (local, domain) = address
        .rsplit_once('@')
        .ok_or_else(|| err!(Request(InvalidParam("Email address must contain a domain"))))?;

    if local.is_empty() || domain.is_empty() {
        return Err!(Request(InvalidParam("Email address is malformed")));
    }

    let local = case_fold(local);
    let domain = case_fold(domain);

    Ok(format!("{local}@{domain}"))
}

fn case_fold(input: &str) -> String {
    input.chars().fold(String::new(), |mut out, c| {
        match c {
            'ß' => out.push_str("ss"),
            other => out.extend(other.to_lowercase()),
        }

        out
    })
}
