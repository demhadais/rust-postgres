use proc_macro2::{Span, TokenStream};
use quote::quote;
use std::iter;
use syn::{
    Data, DataStruct, DeriveInput, Error, Fields, Ident, TraitBound, TraitBoundModifiers,
    TypeParamBound,
};

use crate::accepts;
use crate::composites::CompositeFields;
use crate::composites::{append_generic_bound, new_derive_path};
use crate::enums::Variant;
use crate::overrides::Overrides;

pub fn expand_derive_tosql(input: DeriveInput) -> Result<TokenStream, Error> {
    let overrides = Overrides::extract(&input.attrs, true)?;

    if (overrides.name.is_some() || overrides.rename_all.is_some()) && overrides.transparent {
        return Err(Error::new_spanned(
            &input,
            "#[postgres(transparent)] is not allowed with #[postgres(name = \"...\")] or #[postgres(rename_all = \"...\")]",
        ));
    }

    let name = overrides
        .name
        .clone()
        .unwrap_or_else(|| input.ident.to_string());

    let mut composite_fields = None;

    let (accepts_body, to_sql_body) = if overrides.transparent {
        match input.data {
            Data::Struct(DataStruct {
                fields: Fields::Unnamed(ref fields),
                ..
            }) if fields.unnamed.len() == 1 => {
                let field = fields.unnamed.first().unwrap();

                (accepts::transparent_body(field), transparent_body())
            }
            _ => {
                return Err(Error::new_spanned(
                    input,
                    "#[postgres(transparent)] may only be applied to single field tuple structs",
                ));
            }
        }
    } else if overrides.allow_mismatch {
        match input.data {
            Data::Enum(ref data) => {
                let variants = data
                    .variants
                    .iter()
                    .map(|variant| Variant::parse(variant, overrides.rename_all))
                    .collect::<Result<Vec<_>, _>>()?;
                (
                    accepts::enum_body(&name, &variants, overrides.allow_mismatch),
                    enum_body(&input.ident, &variants),
                )
            }
            _ => {
                return Err(Error::new_spanned(
                    input,
                    "#[postgres(allow_mismatch)] may only be applied to enums",
                ));
            }
        }
    } else {
        match input.data {
            Data::Enum(ref data) => {
                let variants = data
                    .variants
                    .iter()
                    .map(|variant| Variant::parse(variant, overrides.rename_all))
                    .collect::<Result<Vec<_>, _>>()?;
                (
                    accepts::enum_body(&name, &variants, overrides.allow_mismatch),
                    enum_body(&input.ident, &variants),
                )
            }
            Data::Struct(DataStruct {
                fields: Fields::Unnamed(ref fields),
                ..
            }) if fields.unnamed.len() == 1 => {
                let field = fields.unnamed.first().unwrap();

                (accepts::domain_body(&name, field), domain_body())
            }
            Data::Struct(DataStruct {
                fields: Fields::Named(ref fields),
                ..
            }) => {
                composite_fields = Some(CompositeFields::parse(fields, overrides.rename_all)?);
                (accepts::composite_body(&name, "ToSql"), composite_body())
            }
            _ => {
                return Err(Error::new_spanned(
                    input,
                    "#[derive(ToSql)] may only be applied to structs, single field tuple structs, and enums",
                ));
            }
        }
    };

    let ident = &input.ident;
    let generics = append_generic_bound(input.generics.to_owned(), &new_tosql_bound());
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let composite_impl = composite_fields.map(|fields| {
        let body = composite_impl_body(&fields);
        quote! {
            impl #impl_generics postgres_types::ToSqlComposite for #ident #ty_generics #where_clause {
                #body
            }
        }
    });
    let out = quote! {
        impl #impl_generics postgres_types::ToSql for #ident #ty_generics #where_clause {
            fn to_sql(&self,
                      _type: &postgres_types::Type,
                      buf: &mut postgres_types::private::BytesMut)
                      -> std::result::Result<postgres_types::IsNull,
                                             std::boxed::Box<dyn std::error::Error +
                                                             std::marker::Sync +
                                                             std::marker::Send>> {
                #to_sql_body
            }

            fn accepts(type_: &postgres_types::Type) -> bool {
                #accepts_body
            }

            postgres_types::to_sql_checked!();
        }

        #composite_impl
    };

    Ok(out)
}

fn transparent_body() -> TokenStream {
    quote! {
        postgres_types::ToSql::to_sql(&self.0, _type, buf)
    }
}

fn enum_body(ident: &Ident, variants: &[Variant]) -> TokenStream {
    let idents = iter::repeat(ident);
    let variant_idents = variants.iter().map(|v| &v.ident);
    let variant_names = variants.iter().map(|v| &v.name);

    quote! {
        let s = match *self {
            #(
                #idents::#variant_idents => #variant_names,
            )*
        };

        buf.extend_from_slice(s.as_bytes());
        std::result::Result::Ok(postgres_types::IsNull::No)
    }
}

fn domain_body() -> TokenStream {
    quote! {
        let type_ = match *_type.kind() {
            postgres_types::Kind::Domain(ref type_) => type_,
            _ => unreachable!(),
        };

        postgres_types::ToSql::to_sql(&self.0, type_, buf)
    }
}

fn composite_body() -> TokenStream {
    quote! {
        postgres_types::private::composite_to_sql(self, _type, buf)
    }
}

fn composite_impl_body(fields: &CompositeFields) -> TokenStream {
    let num_fields = accepts::composite_num_fields("ToSql", fields);
    let accepts_field_body = accepts::composite_field_body("ToSql", fields);
    let field_names = fields.named.iter().map(|f| &f.name);
    let field_idents = fields.named.iter().map(|f| &f.ident);
    let flattened_idents = fields.flattened.iter().map(|f| &f.ident);

    quote! {
        const NUM_FIELDS: usize = #num_fields;

        fn accepts_field(field: &postgres_types::Field) -> bool {
            #accepts_field_body
        }

        fn field_to_sql(&self,
                        field: &postgres_types::Field,
                        buf: &mut postgres_types::private::BytesMut)
                        -> std::option::Option<
                            std::result::Result<postgres_types::IsNull,
                                                std::boxed::Box<dyn std::error::Error +
                                                                std::marker::Sync +
                                                                std::marker::Send>>> {
            match field.name() {
                #(
                    #field_names => std::option::Option::Some(
                        postgres_types::ToSql::to_sql(&self.#field_idents, field.type_(), buf)),
                )*
                _ => {
                    #(
                        if let std::option::Option::Some(r) =
                            postgres_types::ToSqlComposite::field_to_sql(&self.#flattened_idents, field, buf)
                        {
                            return std::option::Option::Some(r);
                        }
                    )*

                    std::option::Option::None
                }
            }
        }
    }
}

fn new_tosql_bound() -> TypeParamBound {
    TypeParamBound::Trait(TraitBound {
        lifetimes: None,
        modifiers: TraitBoundModifiers::default(),
        maybe: None,
        paren_token: None,
        path: new_derive_path(Ident::new("ToSql", Span::call_site()).into()),
    })
}
