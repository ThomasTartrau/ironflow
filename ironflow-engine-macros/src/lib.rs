//! Derive macros for Ironflow workflow authors.
//!
//! Depend on `ironflow-engine`, not on this crate: it re-exports both derives
//! in `ironflow_engine::decision`, next to the traits they implement, and the
//! generated code refers to `::ironflow_engine`. The derives are documented
//! and tested there.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::ParseStream;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DeriveInput, Error, Field, Fields, Ident, LitStr, Result, Token, Type,
    bracketed, parse_macro_input,
};

/// Derive `ironflow_engine::decision::DecisionAnswers` on a struct whose
/// fields are the questions of a decision step.
///
/// Every field carries exactly one of `#[noul("..")]` (an `f64` probability
/// of "yes", optionally `if_true = ".."` and `if_false = ".."`),
/// `#[choice("..")]` (an enum deriving `DecisionChoice`) or
/// `#[score("..", levels = [".."])]` (an `f64` weighted score). The question
/// is named after the field.
///
/// # Examples
///
/// Compiled and run in `ironflow_engine::decision`:
///
/// ```ignore
/// use ironflow_engine::decision::{DecisionAnswers, DecisionChoice};
///
/// #[derive(DecisionChoice)]
/// enum Team { Billing, Technical }
///
/// #[derive(DecisionAnswers)]
/// struct Triage {
///     #[noul("Does this convey urgency?")]
///     is_urgent: f64,
///     #[choice("Which team should handle this?")]
///     team: Team,
///     #[score("How frustrated?", levels = ["Calm", "Angry"])]
///     mood: f64,
/// }
/// ```
#[proc_macro_derive(DecisionAnswers, attributes(noul, choice, score))]
pub fn derive_decision_answers(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_answers(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Derive `ironflow_engine::decision::DecisionChoice` on an enum whose unit
/// variants are the options of a choice question.
///
/// An option is labelled with its variant name in `snake_case`;
/// `#[choice(rename = "..")]` overrides the label and
/// `#[choice(description = "..")]` tells the model what the option means.
/// Doc comments are never sent to the model.
///
/// # Examples
///
/// Compiled and run in `ironflow_engine::decision`:
///
/// ```ignore
/// use ironflow_engine::decision::DecisionChoice;
///
/// #[derive(DecisionChoice)]
/// enum Team {
///     #[choice(description = "Payments and invoices")]
///     Billing,
///     Technical,
/// }
/// ```
#[proc_macro_derive(DecisionChoice, attributes(choice))]
pub fn derive_decision_choice(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_choice(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// One question, as declared on a field.
enum Question {
    Noul {
        instructions: LitStr,
        if_true: Option<LitStr>,
        if_false: Option<LitStr>,
    },
    Choice {
        instructions: LitStr,
    },
    Score {
        instructions: LitStr,
        levels: Vec<LitStr>,
    },
}

const QUESTION_KINDS: [&str; 3] = ["noul", "choice", "score"];

fn expand_answers(input: &DeriveInput) -> Result<TokenStream2> {
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return Err(Error::new(
                    input.ident.span(),
                    "`DecisionAnswers` derives on a struct with named fields: each field is one question",
                ));
            }
        },
        _ => {
            return Err(Error::new(
                input.ident.span(),
                "`DecisionAnswers` derives on a struct with named fields: each field is one question",
            ));
        }
    };
    if fields.is_empty() {
        return Err(Error::new(
            input.ident.span(),
            "a decision needs at least one question",
        ));
    }

    let support = quote!(::ironflow_engine::decision::__private);
    let mut inserts = Vec::with_capacity(fields.len());
    let mut reads = Vec::with_capacity(fields.len());

    for field in fields {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| Error::new(field.span(), "a question needs a named field"))?;
        let name = ident.unraw().to_string();
        let ty = &field.ty;

        match parse_question(field)? {
            Question::Noul {
                instructions,
                if_true,
                if_false,
            } => {
                let if_true = optional(if_true.as_ref());
                let if_false = optional(if_false.as_ref());
                inserts.push(quote! {
                    questions.insert(
                        ::std::string::String::from(#name),
                        #support::noul(#instructions, #if_true, #if_false),
                    );
                });
                reads.push(read(&support, ident, ty, quote!(read_noul), &name));
            }
            Question::Choice { instructions } => {
                inserts.push(quote_spanned! {ty.span()=>
                    questions.insert(
                        ::std::string::String::from(#name),
                        #support::choice::<#ty>(#instructions),
                    );
                });
                reads.push(read(&support, ident, ty, quote!(read_choice::<#ty>), &name));
            }
            Question::Score {
                instructions,
                levels,
            } => {
                inserts.push(quote! {
                    questions.insert(
                        ::std::string::String::from(#name),
                        #support::score(#instructions, &[#(#levels),*]),
                    );
                });
                reads.push(read(&support, ident, ty, quote!(read_score), &name));
            }
        }
    }

    let type_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::ironflow_engine::decision::DecisionAnswers
            for #type_name #ty_generics #where_clause
        {
            fn questions() -> #support::Questions {
                let mut questions = #support::Questions::new();
                #(#inserts)*
                questions
            }

            fn from_output(
                output: &#support::DecisionOutput,
            ) -> ::core::result::Result<Self, #support::DecisionError> {
                ::core::result::Result::Ok(Self {
                    #(#reads),*
                })
            }
        }
    })
}

/// `field: support::reader(output, "name")?`, spanned on the field type so a
/// type that does not fit the question points at the field.
fn read(
    support: &TokenStream2,
    ident: &Ident,
    ty: &Type,
    reader: TokenStream2,
    name: &str,
) -> TokenStream2 {
    quote_spanned! {ty.span()=>
        #ident: #support::#reader(output, #name)?
    }
}

fn optional(value: Option<&LitStr>) -> TokenStream2 {
    match value {
        Some(value) => quote!(::core::option::Option::Some(#value)),
        None => quote!(::core::option::Option::None),
    }
}

/// The single question attribute of a field.
fn parse_question(field: &Field) -> Result<Question> {
    let attrs: Vec<(&str, &Attribute)> = field
        .attrs
        .iter()
        .filter_map(|attr| {
            QUESTION_KINDS
                .iter()
                .find(|kind| attr.path().is_ident(kind))
                .map(|kind| (*kind, attr))
        })
        .collect();
    let (kind, attr) = match attrs.as_slice() {
        [question] => *question,
        [] => {
            return Err(Error::new(
                field.span(),
                "every field of a `DecisionAnswers` struct is a question: add `#[noul(\"..\")]`, \
                 `#[choice(\"..\")]` or `#[score(\"..\", levels = [..])]`",
            ));
        }
        [_, (_, extra), ..] => {
            return Err(Error::new(
                extra.span(),
                "a field is one question: keep a single `#[noul]`, `#[choice]` or `#[score]`",
            ));
        }
    };

    attr.parse_args_with(|input: ParseStream| parse_question_args(input, kind, attr))
}

fn parse_question_args(input: ParseStream, kind: &str, attr: &Attribute) -> Result<Question> {
    let instructions: LitStr = input.parse()?;
    if instructions.value().trim().is_empty() {
        return Err(Error::new(
            instructions.span(),
            "the instructions of a question must not be empty",
        ));
    }

    let mut if_true = None;
    let mut if_false = None;
    let mut levels: Option<Vec<LitStr>> = None;

    while !input.is_empty() {
        input.parse::<Token![,]>()?;
        if input.is_empty() {
            break;
        }
        let key: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        match (kind, key.to_string().as_str()) {
            ("noul", "if_true") => set_once(&mut if_true, &key, input.parse()?)?,
            ("noul", "if_false") => set_once(&mut if_false, &key, input.parse()?)?,
            ("score", "levels") => {
                let content;
                bracketed!(content in input);
                let parsed = Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?;
                if let Some(blank) = parsed.iter().find(|level| level.value().trim().is_empty()) {
                    return Err(Error::new(blank.span(), "a score level must not be empty"));
                }
                set_once(&mut levels, &key, parsed.into_iter().collect())?;
            }
            _ => {
                return Err(Error::new(
                    key.span(),
                    format!("`#[{kind}]` has no option `{key}`"),
                ));
            }
        }
    }

    match (kind, levels) {
        ("noul", _) => Ok(Question::Noul {
            instructions,
            if_true,
            if_false,
        }),
        ("choice", _) => Ok(Question::Choice { instructions }),
        (_, Some(levels)) if !levels.is_empty() => Ok(Question::Score {
            instructions,
            levels,
        }),
        _ => Err(Error::new(
            attr.span(),
            "`#[score]` needs `levels = [\"..\", ..]`, from the lowest to the highest",
        )),
    }
}

fn set_once<T>(slot: &mut Option<T>, key: &Ident, value: T) -> Result<()> {
    if slot.is_some() {
        return Err(Error::new(key.span(), format!("`{key}` is set twice")));
    }
    *slot = Some(value);
    Ok(())
}

fn expand_choice(input: &DeriveInput) -> Result<TokenStream2> {
    let Data::Enum(data) = &input.data else {
        return Err(Error::new(
            input.ident.span(),
            "`DecisionChoice` derives on an enum: each unit variant is one option",
        ));
    };
    if data.variants.is_empty() {
        return Err(Error::new(
            input.ident.span(),
            "a choice needs at least one option",
        ));
    }

    let mut variants = Vec::with_capacity(data.variants.len());
    let mut labels: Vec<String> = Vec::with_capacity(data.variants.len());
    let mut descriptions = Vec::with_capacity(data.variants.len());

    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(Error::new(
                variant.span(),
                "`DecisionChoice` options are unit variants",
            ));
        }

        let mut rename: Option<LitStr> = None;
        let mut description: Option<LitStr> = None;
        for attr in variant.attrs.iter().filter(|a| a.path().is_ident("choice")) {
            attr.parse_nested_meta(|meta| {
                let slot = if meta.path.is_ident("rename") {
                    &mut rename
                } else if meta.path.is_ident("description") {
                    &mut description
                } else {
                    return Err(meta.error("expected `rename` or `description`"));
                };
                let value: LitStr = meta.value()?.parse()?;
                if value.value().trim().is_empty() {
                    return Err(Error::new(value.span(), "must not be empty"));
                }
                if slot.is_some() {
                    return Err(meta.error("set twice"));
                }
                *slot = Some(value);
                Ok(())
            })?;
        }

        let label = rename
            .map(|lit| lit.value())
            .unwrap_or_else(|| snake_case(&variant.ident.unraw().to_string()));
        if labels.contains(&label) {
            return Err(Error::new(
                variant.span(),
                format!("two options are labelled `{label}`"),
            ));
        }

        variants.push(&variant.ident);
        labels.push(label);
        descriptions.push(optional(description.as_ref()));
    }

    let type_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::ironflow_engine::decision::DecisionChoice
            for #type_name #ty_generics #where_clause
        {
            fn options() -> ::std::vec::Vec<(&'static str, ::core::option::Option<&'static str>)> {
                ::std::vec![#((#labels, #descriptions)),*]
            }

            fn from_label(label: &str) -> ::core::option::Option<Self> {
                match label {
                    #(#labels => ::core::option::Option::Some(Self::#variants),)*
                    _ => ::core::option::Option::None,
                }
            }

            fn label(&self) -> &'static str {
                match self {
                    #(Self::#variants => #labels,)*
                }
            }
        }
    })
}

/// `OnCallSRE` -> `on_call_sre`, `HTTPError` -> `http_error`.
fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_uppercase() {
            let prev = i.checked_sub(1).map(|p| chars[p]);
            let next = chars.get(i + 1).copied();
            let starts_word = match prev {
                None => false,
                Some(prev) => {
                    prev.is_lowercase()
                        || prev.is_ascii_digit()
                        || (prev.is_uppercase() && next.is_some_and(char::is_lowercase))
                }
            };
            if starts_word {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_splits_words() {
        assert_eq!(snake_case("Billing"), "billing");
        assert_eq!(snake_case("OnCall"), "on_call");
        assert_eq!(snake_case("OnCallSRE"), "on_call_sre");
        assert_eq!(snake_case("HTTPError"), "http_error");
        assert_eq!(snake_case("Tier2Support"), "tier2_support");
        assert_eq!(snake_case("Été"), "été");
    }

    #[test]
    fn snake_case_keeps_an_already_snake_name() {
        assert_eq!(snake_case("billing"), "billing");
        assert_eq!(snake_case("on_call"), "on_call");
    }
}
