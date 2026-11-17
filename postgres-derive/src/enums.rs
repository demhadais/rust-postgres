use syn::{Error, Fields, Ident};

use crate::{case::RenameRule, overrides::Overrides};

pub struct Variant {
    pub ident: Ident,
    pub name: String,
}

impl Variant {
    pub fn parse(raw: &syn::Variant, rename_all: Option<RenameRule>) -> Result<Variant, Error> {
        match raw.fields {
            Fields::Unit => {}
            _ => {
                return Err(Error::new_spanned(
                    raw,
                    "non-C-like enums are not supported",
                ));
            }
        }
        let overrides = Overrides::extract(&raw.attrs, false)?;

        if overrides.flatten {
            return Err(Error::new_spanned(
                raw,
                "#[postgres(flatten)] may only be applied to struct fields",
            ));
        }

        // variant level name override takes precendence over container level rename_all override
        let name = overrides.name.unwrap_or_else(|| match rename_all {
            Some(rule) => rule.apply_to_field(&raw.ident.to_string()),
            None => raw.ident.to_string(),
        });
        Ok(Variant {
            ident: raw.ident.clone(),
            name,
        })
    }
}
