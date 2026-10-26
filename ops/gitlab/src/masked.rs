//! [`Masked`] -- an endpoint whose request body has its secrets masked.

use std::borrow::Cow;
use std::str;

use form_urlencoded::{Serializer, parse};
use gitlab::api::endpoint_prelude::{BodyError, Endpoint, Method, QueryParams, UrlBase};
use ironflow_ops_common::redact::Redactor;
use serde_json::{Value, from_slice};

/// Wraps an endpoint and masks the secrets of its body, leaving the method,
/// path and query parameters untouched. Without a redactor, the body is sent
/// as is.
pub(crate) struct Masked<'a, E> {
    pub(crate) inner: &'a E,
    pub(crate) redactor: Option<&'a Redactor>,
}

impl<E: Endpoint> Endpoint for Masked<'_, E> {
    fn method(&self) -> Method {
        self.inner.method()
    }

    fn endpoint(&self) -> Cow<'static, str> {
        self.inner.endpoint()
    }

    fn url_base(&self) -> UrlBase {
        self.inner.url_base()
    }

    fn parameters(&self) -> QueryParams<'_> {
        self.inner.parameters()
    }

    fn body(&self) -> Result<Option<(&'static str, Vec<u8>)>, BodyError> {
        let Some(redactor) = self.redactor else {
            return self.inner.body();
        };
        Ok(self.inner.body()?.map(|(content_type, body)| {
            let masked = mask_body(redactor, content_type, &body);
            (content_type, masked.unwrap_or(body))
        }))
    }
}

/// The masked body, or `None` when it carries no secret.
///
/// A form or a JSON body is decoded first: the encoding escapes characters a
/// secret may hold (`+`, `/`, `=`, `"`), so a search on the encoded bytes
/// would miss it. A `text/*` body is masked as text. Any other body is file
/// content (a generic package, a project import archive) and is sent as is:
/// rewriting it would change its checksum.
pub(crate) fn mask_body(redactor: &Redactor, content_type: &str, body: &[u8]) -> Option<Vec<u8>> {
    match content_type {
        "application/x-www-form-urlencoded" => mask_form(redactor, body),
        "application/json" => match from_slice::<Value>(body) {
            Ok(mut value) => {
                mask_json(redactor, &mut value).then(|| value.to_string().into_bytes())
            }
            Err(_) => mask_text(redactor, body),
        },
        text if text.starts_with("text/") => mask_text(redactor, body),
        _ => None,
    }
}

fn mask_form(redactor: &Redactor, body: &[u8]) -> Option<Vec<u8>> {
    let mut masked = false;
    let pairs: Vec<(String, String)> = parse(body)
        .map(|(key, value)| {
            let value = match redactor.redact(&value) {
                Some(redacted) => {
                    masked = true;
                    redacted
                }
                None => value.into_owned(),
            };
            (key.into_owned(), value)
        })
        .collect();
    masked.then(|| {
        Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish()
            .into_bytes()
    })
}

/// Masks every string of `value` in place. Returns whether anything was masked.
fn mask_json(redactor: &Redactor, value: &mut Value) -> bool {
    match value {
        Value::String(text) => match redactor.redact(text) {
            Some(redacted) => {
                *text = redacted;
                true
            }
            None => false,
        },
        Value::Array(items) => items
            .iter_mut()
            .fold(false, |masked, item| mask_json(redactor, item) | masked),
        Value::Object(fields) => fields
            .values_mut()
            .fold(false, |masked, field| mask_json(redactor, field) | masked),
        _ => false,
    }
}

fn mask_text(redactor: &Redactor, body: &[u8]) -> Option<Vec<u8>> {
    let text = str::from_utf8(body).ok()?;
    redactor.redact(text).map(String::into_bytes)
}

#[cfg(test)]
mod tests {
    use ironflow_ops_common::redact::Redactor;

    use super::mask_body;

    const GLPAT: &str = "glpat-abcdefghij0123456789";

    #[test]
    fn invalid_json_body_is_masked_as_text() {
        let body = format!("{{\"body\": \"{GLPAT}\"");
        let masked = mask_body(&Redactor::default(), "application/json", body.as_bytes());
        assert_eq!(masked.as_deref(), Some(&b"{\"body\": \"[REDACTED]\""[..]));
    }

    #[test]
    fn package_content_is_sent_as_is_even_when_it_is_text() {
        let body = format!("fixture token {GLPAT}\n");
        assert_eq!(
            mask_body(
                &Redactor::default(),
                "application/octet-stream",
                body.as_bytes()
            ),
            None
        );
    }

    #[test]
    fn text_body_that_is_not_utf8_is_sent_as_is() {
        let mut body = vec![0xff, 0xfe];
        body.extend_from_slice(GLPAT.as_bytes());
        assert_eq!(mask_body(&Redactor::default(), "text/plain", &body), None);
    }

    #[test]
    fn form_body_keeps_its_other_fields() {
        let body = format!("title=fix&description=see+{GLPAT}&squash=true");
        let masked = mask_body(
            &Redactor::default(),
            "application/x-www-form-urlencoded",
            body.as_bytes(),
        );
        assert_eq!(
            masked.as_deref(),
            Some(&b"title=fix&description=see+%5BREDACTED%5D&squash=true"[..])
        );
    }
}
