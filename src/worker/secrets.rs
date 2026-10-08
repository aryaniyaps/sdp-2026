//! A claim that contains a credential is never stored, whatever the model decided.

fn token_chars(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
        .filter(|token| !token.is_empty())
}

/// True when the text carries a key, token, private key block or a password in a URL.
pub fn leaks_secret(text: &str) -> bool {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY-----") {
        return true;
    }
    if let Some(scheme) = text.find("://") {
        let rest = &text[scheme + 3..];
        let authority = rest.split(['/', ' ', '\n']).next().unwrap_or("");
        if let Some((credentials, _)) = authority.split_once('@') {
            if credentials.split_once(':').is_some_and(|(_, pw)| pw.len() >= 4) {
                return true;
            }
        }
    }
    token_chars(text).any(|token| {
        let body = |prefix: &str, min: usize| {
            token.strip_prefix(prefix).is_some_and(|rest| rest.len() >= min)
        };
        body("sk-", 16)
            || body("ghp_", 20)
            || (token.starts_with("AKIA")
                && token.len() == 20
                && token.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
            || (["xoxa-", "xoxb-", "xoxp-"].iter().any(|p| body(p, 10)))
            || (token.starts_with("eyJ") && token.matches('.').count() >= 2 && token.len() > 30)
    })
}

#[cfg(test)]
mod tests {
    use super::leaks_secret;

    #[test]
    fn catches_credentials() {
        for text in [
            "The developer provided the live key sk-live-4eC39HqLyjWDarjtT1zdp7dcX.",
            "token ghp_abcdefghijklmnopqrstuvwxyz0123",
            "aws AKIAABCDEFGHIJKLMNOP is set",
            "connect with postgres://admin:hunter22@db.internal/app",
            "header eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.sig",
            "-----BEGIN RSA PRIVATE KEY-----",
        ] {
            assert!(leaks_secret(text), "{text}");
        }
    }

    #[test]
    fn keeps_ordinary_facts() {
        for text in [
            "The team deploys on Thursdays at 17:00 IST.",
            "The project uses pnpm and Node 22.",
            "Use the sk- prefix naming for skills.",
            "Database is postgres://db.internal/app",
        ] {
            assert!(!leaks_secret(text), "{text}");
        }
    }
}
