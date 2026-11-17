use proc_macro2::{Span, TokenStream};
use quote::quote;
use std::iter;
use syn::{
    AngleBracketedGenericArguments, Data, DataStruct, DeriveInput, Error, Fields, GenericArgument,
    GenericParam, Generics, Ident, Lifetime, PathArguments, PathSegment, punctuated::Punctuated,
    token,
};
use syn::{LifetimeParam, TraitBound, TraitBoundModifiers, TypeParamBound};

use crate::accepts;
use crate::composites::CompositeFields;
use crate::composites::{append_generic_bound, new_derive_path};
use crate::enums::Variant;
use crate::overrides::Overrides;

pub fn expand_derive_fromsql(input: DeriveInput) -> Result<TokenStream, Error> {
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
                (
                    accepts::transparent_body(field),
                    transparent_body(&input.ident, field),
                )
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
                (
                    domain_accepts_body(&name, field),
                    domain_body(&input.ident, field),
                )
            }
            Data::Struct(DataStruct {
                fields: Fields::Named(ref fields),
                ..
            }) => {
                composite_fields = Some(CompositeFields::parse(fields, overrides.rename_all)?);
                (accepts::composite_body(&name, "FromSql"), composite_body())
            }
            _ => {
                return Err(Error::new_spanned(
                    input,
                    "#[derive(FromSql)] may only be applied to structs, single field tuple structs, and enums",
                ));
            }
        }
    };

    let ident = &input.ident;
    let (generics, lifetime) = build_generics(&input.generics);
    let (impl_generics, _, _) = generics.split_for_impl();
    let (_, ty_generics, where_clause) = input.generics.split_for_impl();
    let composite_impl = composite_fields.map(|fields| {
        let body = composite_impl_body(ident, &fields, &lifetime);
        quote! {
            impl #impl_generics postgres_types::private::FromSqlComposite<#lifetime> for #ident #ty_generics #where_clause {
                #body
            }
        }
    });
    let out = quote! {
        impl #impl_generics postgres_types::FromSql<#lifetime> for #ident #ty_generics #where_clause {
            fn from_sql(_type: &postgres_types::Type, buf: &#lifetime [u8])
                        -> std::result::Result<#ident #ty_generics,
                                               std::boxed::Box<dyn std::error::Error +
                                                               std::marker::Sync +
                                                               std::marker::Send>> {
                #to_sql_body
            }

            fn accepts(type_: &postgres_types::Type) -> bool {
                #accepts_body
            }
        }

        #composite_impl
    };

    Ok(out)
}

fn transparent_body(ident: &Ident, field: &syn::Field) -> TokenStream {
    let ty = &field.ty;
    quote! {
        <#ty as postgres_types::FromSql>::from_sql(_type, buf).map(#ident)
    }
}

fn enum_body(ident: &Ident, variants: &[Variant]) -> TokenStream {
    let variant_names = variants.iter().map(|v| &v.name);
    let idents = iter::repeat(ident);
    let variant_idents = variants.iter().map(|v| &v.ident);

    quote! {
        match std::str::from_utf8(buf)? {
            #(
                #variant_names => std::result::Result::Ok(#idents::#variant_idents),
            )*
            s => {
                std::result::Result::Err(
                    std::convert::Into::into(format!("invalid variant `{}`", s)))
            }
        }
    }
}

// Domains are sometimes but not always just represented by the bare type (!?)
fn domain_accepts_body(name: &str, field: &syn::Field) -> TokenStream {
    let ty = &field.ty;
    let normal_body = accepts::domain_body(name, field);

    quote! {
        if <#ty as postgres_types::FromSql>::accepts(type_) {
            return true;
        }

        #normal_body
    }
}

fn domain_body(ident: &Ident, field: &syn::Field) -> TokenStream {
    let ty = &field.ty;
    quote! {
        <#ty as postgres_types::FromSql>::from_sql(match *_type.kind() {
            postgres_types::Kind::Domain(ref _type) => _type,
            _ => _type
        }, buf).map(#ident)
    }
}

fn composite_body() -> TokenStream {
    quote! {
        postgres_types::private::composite_from_sql(_type, buf)
    }
}

fn composite_impl_body(
    ident: &Ident,
    fields: &CompositeFields,
    lifetime: &Lifetime,
) -> TokenStream {
    let num_fields = accepts::composite_num_fields("FromSql", fields);
    let accepts_field_body = accepts::composite_field_body("FromSql", fields);
    let field_names = fields.named.iter().map(|f| &f.name);
    let field_idents = fields.named.iter().map(|f| &f.ident);
    let flattened_idents = fields.flattened.iter().map(|f| &f.ident);
    let flattened_types = fields.flattened.iter().map(|f| &f.type_);

    quote! {
        const NUM_FIELDS: usize = #num_fields;

        fn accepts_field(field: &postgres_types::Field) -> bool {
            #accepts_field_body
        }

        fn from_sql_fields(reader: &mut postgres_types::private::CompositeReader<'_, #lifetime>)
                           -> std::result::Result<Self,
                                                  std::boxed::Box<dyn std::error::Error +
                                                                  std::marker::Sync +
                                                                  std::marker::Send>> {
            std::result::Result::Ok(#ident {
                #(
                    #field_idents: reader.get(#field_names)?,
                )*
                #(
                    #flattened_idents: <#flattened_types as postgres_types::private::FromSqlComposite<#lifetime>>::from_sql_fields(reader)?,
                )*
            })
        }
    }
}

fn build_generics(source: &Generics) -> (Generics, Lifetime) {
    // don't worry about lifetime name collisions, it doesn't make sense to derive FromSql on a struct with a lifetime
    let lifetime = Lifetime::new("'a", Span::call_site());

    let mut out = append_generic_bound(source.to_owned(), &new_fromsql_bound(&lifetime));
    out.params.insert(
        0,
        GenericParam::Lifetime(LifetimeParam::new(lifetime.to_owned())),
    );

    (out, lifetime)
}

fn new_fromsql_bound(lifetime: &Lifetime) -> TypeParamBound {
    let mut path_segment: PathSegment = Ident::new("FromSql", Span::call_site()).into();
    let mut seg_args = Punctuated::new();
    seg_args.push(GenericArgument::Lifetime(lifetime.to_owned()));
    path_segment.arguments = PathArguments::AngleBracketed(AngleBracketedGenericArguments {
        colon2_token: None,
        lt_token: token::Lt::default(),
        args: seg_args,
        gt_token: token::Gt::default(),
    });

    TypeParamBound::Trait(TraitBound {
        lifetimes: None,
        modifiers: TraitBoundModifiers::default(),
        maybe: None,
        paren_token: None,
        path: new_derive_path(path_segment),
    })
}
