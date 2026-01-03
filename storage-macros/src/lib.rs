//! Procedural macros for storage address validation at compile time.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Expr, Ident, Lit};

/// Derives compile-time validation for storage address spaces and generates helper methods.
///
/// This macro:
/// 1. Validates at compile time that each variant's start address <= end address
/// 2. Validates at compile time that no address ranges overlap between variants
/// 3. Generates a `get_address(&self) -> (u32, u32)` method returning (start, end)
/// 4. Generates an `is_address_reserved(&self) -> bool` method
///
/// # Usage
///
/// ```ignore
/// #[derive(AddrSpace)]
/// pub enum StorageContents {
///     #[addr_space(0x0000, 0x8fff, reserved)]
///     ReservedStart,
///     #[addr_space(0x9000, 0x9000)]
///     FirstFrameMetadata,
///     // ...
/// }
///
/// // Generated methods:
/// // impl StorageContents {
/// //     pub fn get_address(&self) -> (u32, u32) { ... }
/// //     pub fn is_address_reserved(&self) -> bool { ... }
/// // }
/// ```
#[proc_macro_derive(AddrSpace, attributes(addr_space))]
pub fn addr_space_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let enum_name = &input.ident;

    let Data::Enum(data_enum) = &input.data else {
        return syn::Error::new(Span::call_site(), "AddrSpace can only be applied to enums")
            .to_compile_error()
            .into();
    };

    // Collect all address ranges with their variant names, identifiers, and reserved flag
    let mut ranges: Vec<(Ident, u64, u64, bool)> = Vec::new();

    for variant in &data_enum.variants {
        let variant_ident = variant.ident.clone();
        let variant_name = variant_ident.to_string();

        // Look for #[addr_space(start, end)] or #[addr_space(start, end, reserved)] attribute
        let addr_attr = variant.attrs.iter().find(|attr| attr.path().is_ident("addr_space"));

        let Some(attr) = addr_attr else {
            return syn::Error::new_spanned(
                &variant.ident,
                format!("Variant `{}` is missing #[addr_space(start, end)] attribute", variant_name),
            )
            .to_compile_error()
            .into();
        };

        // Parse the attribute arguments: (start, end) or (start, end, reserved)
        let result: Result<(u64, u64, bool), syn::Error> = attr.parse_args_with(|input: syn::parse::ParseStream| {
            let start: Expr = input.parse()?;
            input.parse::<syn::Token![,]>()?;
            let end: Expr = input.parse()?;

            let start_val = parse_int_literal(&start)?;
            let end_val = parse_int_literal(&end)?;

            // Check for optional ", reserved" at the end
            let is_reserved = if input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
                let ident: Ident = input.parse()?;
                if ident != "reserved" {
                    return Err(syn::Error::new_spanned(
                        ident,
                        "Expected 'reserved' keyword",
                    ));
                }
                true
            } else {
                false
            };

            Ok((start_val, end_val, is_reserved))
        });

        let (start, end, is_reserved) = match result {
            Ok(v) => v,
            Err(e) => return e.to_compile_error().into(),
        };

        // Validate start <= end
        if start > end {
            return syn::Error::new_spanned(
                attr,
                format!(
                    "Variant `{}`: start address (0x{:x}) must be <= end address (0x{:x})",
                    variant_name, start, end
                ),
            )
            .to_compile_error()
            .into();
        }

        ranges.push((variant_ident, start, end, is_reserved));
    }

    // Check for overlapping ranges
    for (i, (name_i, start_i, end_i, _)) in ranges.iter().enumerate() {
        for (name_j, start_j, end_j, _) in ranges.iter().skip(i + 1) {
            // Two ranges overlap if one starts before the other ends
            let overlaps = start_i <= end_j && start_j <= end_i;

            if overlaps {
                return syn::Error::new(
                    Span::call_site(),
                    format!(
                        "Address ranges overlap:\n  `{}`: 0x{:x}..=0x{:x}\n  `{}`: 0x{:x}..=0x{:x}",
                        name_i, start_i, end_i, name_j, start_j, end_j
                    ),
                )
                .to_compile_error()
                .into();
            }
        }
    }

    // Generate match arms for get_address()
    let address_match_arms = ranges.iter().map(|(variant_ident, start, end, _)| {
        let start_lit = *start as u32;
        let end_lit = *end as u32;
        quote! {
            Self::#variant_ident => (#start_lit, #end_lit),
        }
    });

    // Generate match arms for is_address_reserved()
    let reserved_match_arms = ranges.iter().map(|(variant_ident, _, _, is_reserved)| {
        quote! {
            Self::#variant_ident => #is_reserved,
        }
    });

    // Generate the impl block with both methods
    let expanded = quote! {
        impl #enum_name {
            /// Returns the (start, end) address range for this storage content type.
            /// Both addresses are inclusive.
            pub fn get_address(&self) -> (u32, u32) {
                match self {
                    #(#address_match_arms)*
                }
            }

            /// Returns whether this address range is reserved (cannot be written to).
            pub fn is_address_reserved(&self) -> bool {
                match self {
                    #(#reserved_match_arms)*
                }
            }
        }
    };

    TokenStream::from(expanded)
}

fn parse_int_literal(expr: &Expr) -> Result<u64, syn::Error> {
    match expr {
        Expr::Lit(expr_lit) => {
            if let Lit::Int(lit_int) = &expr_lit.lit {
                lit_int.base10_parse::<u64>()
            } else {
                Err(syn::Error::new_spanned(expr, "Expected integer literal"))
            }
        }
        _ => Err(syn::Error::new_spanned(expr, "Expected integer literal")),
    }
}
