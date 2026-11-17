use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::iter;
use syn::Ident;

use crate::composites::CompositeFields;
use crate::enums::Variant;

pub fn transparent_body(field: &syn::Field) -> TokenStream {
    let ty = &field.ty;

    quote! {
        <#ty as ::postgres_types::ToSql>::accepts(type_)
    }
}

pub fn domain_body(name: &str, field: &syn::Field) -> TokenStream {
    let ty = &field.ty;

    quote! {
        if type_.name() != #name {
            return false;
        }

        match *type_.kind() {
            ::postgres_types::Kind::Domain(ref type_) => {
                <#ty as ::postgres_types::ToSql>::accepts(type_)
            }
            _ => false,
        }
    }
}

pub fn enum_body(name: &str, variants: &[Variant], allow_mismatch: bool) -> TokenStream {
    let num_variants = variants.len();
    let variant_names = variants.iter().map(|v| &v.name);

    if allow_mismatch {
        quote! {
            type_.name() == #name
        }
    } else {
        quote! {
            if type_.name() != #name {
                return false;
            }

            match *type_.kind() {
                ::postgres_types::Kind::Enum(ref variants) => {
                    if variants.len() != #num_variants {
                        return false;
                    }

                    variants.iter().all(|v| {
                        match &**v {
                            #(
                                #variant_names => true,
                            )*
                            _ => false,
                        }
                    })
                }
                _ => false,
            }
        }
    }
}

pub fn composite_body(name: &str, trait_: &str) -> TokenStream {
    let composite_trait = format_ident!("{}Composite", trait_);

    quote! {
        ::postgres_types::private::composite_accepts(
            type_,
            #name,
            <Self as ::postgres_types::private::#composite_trait>::NUM_FIELDS,
            <Self as ::postgres_types::private::#composite_trait>::accepts_field,
        )
    }
}

pub fn composite_num_fields(trait_: &str, fields: &CompositeFields) -> TokenStream {
    let composite_trait = format_ident!("{}Composite", trait_);
    let num_named = fields.named.len();
    let flattened_types = fields.flattened.iter().map(|f| &f.type_);

    quote! {
        #num_named #(
            + <#flattened_types as ::postgres_types::private::#composite_trait>::NUM_FIELDS
        )*
    }
}

pub fn composite_field_body(trait_: &str, fields: &CompositeFields) -> TokenStream {
    let composite_trait = format_ident!("{}Composite", trait_);
    let trait_ = Ident::new(trait_, Span::call_site());
    let traits = iter::repeat(&trait_);
    let field_names = fields.named.iter().map(|f| &f.name);
    let field_types = fields.named.iter().map(|f| &f.type_);
    let flattened_types = fields.flattened.iter().map(|f| &f.type_);

    quote! {
        match field.name() {
            #(
                #field_names => {
                    <#field_types as ::postgres_types::#traits>::accepts(field.type_())
                }
            )*
            _ => {
                false #(
                    || <#flattened_types as ::postgres_types::private::#composite_trait>::accepts_field(field)
                )*
            }
        }
    }
}
