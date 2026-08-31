use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{parse_macro_input, spanned::Spanned, Data, DeriveInput, Field, Fields, LitStr, Token, Type};

/// Derives the PultSchema marker for an entity struct.
///
/// Supported field attributes:
///   #[pult(lifecycle = LOCAL | SYNCED | PERSISTED)]
///   #[pult(primary_key)]
///   #[pult(collection_of = TypeName, ordered)]   (on Vec<Uuid> fields)
///
/// Supported struct attributes:
///   #[pult(table = "table_name")]
///
/// Generated items:
///   - PultEntity impl
///   - {T}Patch, {T}Create, {T}Accessor structs
///   - PultSqlRow impl (entities with PERSISTED fields + table)
///   - inventory::submit!(EntityMeta { ... })
#[proc_macro_derive(PultSchema, attributes(pult))]
pub fn derive_pult_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive_impl(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Attribute macro on an impl block containing #[pult_command] methods.
///
/// For each annotated method generates a handler fn + inventory::submit!(CommandRegistration).
/// Strips #[pult_command] attributes so the compiler never sees them.
///
/// Expected method signatures:
///   fn name(&mut self) -> Result<(), E>
///   fn name(&mut self, args: serde_json::Value) -> Result<(), E>
#[proc_macro_attribute]
pub fn pult_commands(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut impl_block = parse_macro_input!(item as syn::ItemImpl);
    match pult_commands_impl(&mut impl_block) {
        Ok(extra) => quote! { #impl_block #extra }.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// ── Parsed representation ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Lifecycle {
    Local,
    Synced,
    Persisted,
}

struct FieldMeta {
    ident: syn::Ident,
    ty: Type,
    lifecycle: Lifecycle,
    is_primary_key: bool,
    collection_of: Option<syn::Ident>,
    /// From `#[pult(ordered)]`. Parsed and carried but not yet acted on: collection
    /// order is not persisted, so nothing consumes it.
    #[allow(dead_code)]
    is_ordered: bool,
}

struct StructMeta {
    ident: syn::Ident,
    table_name: Option<String>,
    is_singleton: bool,
    fields: Vec<FieldMeta>,
}

// ── Parsing helpers ───────────────────────────────────────────────────────────

fn parse_struct_meta(input: &DeriveInput) -> syn::Result<StructMeta> {
    let mut table_name = None;
    let mut is_singleton = false;

    for attr in &input.attrs {
        if !attr.path().is_ident("pult") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                let _: Token![=] = meta.input.parse()?;
                let lit: LitStr = meta.input.parse()?;
                table_name = Some(lit.value());
                Ok(())
            } else if meta.path.is_ident("singleton") {
                is_singleton = true;
                Ok(())
            } else {
                Err(meta.error("unknown pult struct attribute"))
            }
        })?;
    }

    let named_fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => f,
            _ => return Err(syn::Error::new(input.span(), "PultSchema requires named fields")),
        },
        _ => return Err(syn::Error::new(input.span(), "PultSchema requires a struct")),
    };

    let fields = named_fields
        .named
        .iter()
        .map(parse_field_meta)
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(StructMeta { ident: input.ident.clone(), table_name, is_singleton, fields })
}

fn parse_field_meta(field: &Field) -> syn::Result<FieldMeta> {
    let ident = field.ident.clone().ok_or_else(|| syn::Error::new(field.span(), "unnamed field"))?;
    let ty = field.ty.clone();

    let mut lifecycle = None;
    let mut is_primary_key = false;
    let mut collection_of = None;
    let mut is_ordered = false;

    for attr in &field.attrs {
        if !attr.path().is_ident("pult") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("lifecycle") {
                let _: Token![=] = meta.input.parse()?;
                let ident: syn::Ident = meta.input.parse()?;
                lifecycle = Some(match ident.to_string().as_str() {
                    "LOCAL" => Lifecycle::Local,
                    "SYNCED" => Lifecycle::Synced,
                    "PERSISTED" => Lifecycle::Persisted,
                    other => return Err(syn::Error::new(ident.span(), format!("unknown lifecycle: {other}"))),
                });
                Ok(())
            } else if meta.path.is_ident("primary_key") {
                is_primary_key = true;
                Ok(())
            } else if meta.path.is_ident("collection_of") {
                let _: Token![=] = meta.input.parse()?;
                let name: syn::Ident = meta.input.parse()?;
                collection_of = Some(name);
                Ok(())
            } else if meta.path.is_ident("ordered") {
                is_ordered = true;
                Ok(())
            } else {
                Err(meta.error("unknown pult field attribute"))
            }
        })?;
    }

    let lifecycle =
        lifecycle.ok_or_else(|| syn::Error::new(field.span(), "field missing #[pult(lifecycle = ...)]"))?;

    Ok(FieldMeta { ident, ty, lifecycle, is_primary_key, collection_of, is_ordered })
}

// ── Code generation ───────────────────────────────────────────────────────────

fn derive_impl(input: DeriveInput) -> syn::Result<TokenStream2> {
    let meta = parse_struct_meta(&input)?;

    let entity_impl = gen_pult_entity(&meta)?;
    let patch_struct = gen_patch_struct(&meta)?;
    let create_struct = gen_create_struct(&meta)?;
    let accessor_struct = gen_accessor_struct(&meta)?;
    let sql_row_impl = gen_pult_sql_row(&meta)?;
    let entity_meta_submit = gen_entity_meta_submit(&meta)?;

    Ok(quote! {
        #entity_impl
        #patch_struct
        #create_struct
        #accessor_struct
        #sql_row_impl
        #entity_meta_submit
    })
}

// ── PultEntity impl ───────────────────────────────────────────────────────────

fn gen_pult_entity(meta: &StructMeta) -> syn::Result<TokenStream2> {
    let name = &meta.ident;
    let table = match &meta.table_name {
        Some(t) => quote! { Some(#t) },
        None => quote! { None },
    };
    let pk = match meta.fields.iter().find(|f| f.is_primary_key) {
        Some(f) => {
            let n = f.ident.to_string();
            quote! { Some(#n) }
        }
        None => quote! { None },
    };

    let lifecycle_entries: Vec<_> = meta
        .fields
        .iter()
        .map(|f| {
            let name_str = f.ident.to_string();
            let lc = match f.lifecycle {
                Lifecycle::Local => quote! { ::pult_schema::lifecycle::Lifecycle::Local },
                Lifecycle::Synced => quote! { ::pult_schema::lifecycle::Lifecycle::Synced },
                Lifecycle::Persisted => quote! { ::pult_schema::lifecycle::Lifecycle::Persisted },
            };
            quote! { (#name_str, #lc) }
        })
        .collect();

    Ok(quote! {
        impl ::pult_schema::traits::PultEntity for #name {
            fn table_name() -> Option<&'static str> { #table }
            fn primary_key_field() -> Option<&'static str> { #pk }
            fn field_lifecycles() -> &'static [(&'static str, ::pult_schema::lifecycle::Lifecycle)] {
                &[ #(#lifecycle_entries),* ]
            }
        }
    })
}

// ── {T}Patch struct ───────────────────────────────────────────────────────────

fn gen_patch_struct(meta: &StructMeta) -> syn::Result<TokenStream2> {
    let patch_name = format_ident!("{}Patch", meta.ident);

    let patch_fields: Vec<_> = meta
        .fields
        .iter()
        .filter(|f| f.lifecycle != Lifecycle::Local)
        .map(|f| {
            let ident = &f.ident;
            let ty = &f.ty;
            quote! {
                #[serde(skip_serializing_if = "Option::is_none")]
                pub #ident: Option<#ty>
            }
        })
        .collect();

    Ok(quote! {
        #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize, ::ts_rs::TS)]
        #[ts(export)]
        pub struct #patch_name {
            #(#patch_fields,)*
        }
    })
}

// ── {T}Create struct ──────────────────────────────────────────────────────────

fn gen_create_struct(meta: &StructMeta) -> syn::Result<TokenStream2> {
    let create_name = format_ident!("{}Create", meta.ident);

    let create_fields: Vec<_> = meta
        .fields
        .iter()
        .filter(|f| f.lifecycle == Lifecycle::Persisted && !f.is_primary_key)
        .map(|f| {
            let ident = &f.ident;
            let ty = &f.ty;
            quote! { pub #ident: #ty }
        })
        .collect();

    Ok(quote! {
        #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize, ::ts_rs::TS)]
        #[ts(export)]
        pub struct #create_name {
            #(#create_fields,)*
        }
    })
}

// ── {T}Accessor struct ────────────────────────────────────────────────────────

fn gen_accessor_struct(meta: &StructMeta) -> syn::Result<TokenStream2> {
    let accessor_name = format_ident!("{}Accessor", meta.ident);
    let entity_name = &meta.ident;

    let field_methods: Vec<_> = meta
        .fields
        .iter()
        .filter(|f| f.collection_of.is_none())
        .map(gen_field_accessor_method)
        .collect::<syn::Result<Vec<_>>>()?;

    let collection_methods: Vec<_> = meta
        .fields
        .iter()
        .filter(|f| f.collection_of.is_some())
        .map(gen_collection_accessor_method)
        .collect::<syn::Result<Vec<_>>>()?;

    let create_name = format_ident!("{}Create", entity_name);

    // Reach this entity from the root: `data.sequences()`, `data.show()`. The method
    // name and the path key are both the table name, so the root has nothing to keep
    // in step by hand.
    let root_method = match &meta.table_name {
        None => quote! {},
        Some(table) => {
            let method = format_ident!("{}", table);
            let doc = format!("Access the `{table}` {}.", if meta.is_singleton { "singleton" } else { "collection" });
            if meta.is_singleton {
                quote! {
                    impl<H: ::pult_schema::handle::DataHandle> ::pult_schema::handle::ShowDataRoot<H> {
                        #[doc = #doc]
                        pub fn #method(&self) -> #accessor_name<H> {
                            <#accessor_name<H> as ::pult_schema::handle::EntityAccessor>::new(
                                vec![::pult_schema::path::PathSegment::Key(#table.into())],
                                self.handle.clone(),
                            )
                        }
                    }
                }
            } else {
                quote! {
                    impl<H: ::pult_schema::handle::DataHandle> ::pult_schema::handle::ShowDataRoot<H> {
                        #[doc = #doc]
                        pub fn #method(&self) -> ::pult_schema::handle::EntityCollectionAccessor<#accessor_name<H>> {
                            ::pult_schema::handle::EntityCollectionAccessor::new(
                                vec![::pult_schema::path::PathSegment::Key(#table.into())],
                                self.handle.clone(),
                            )
                        }
                    }
                }
            }
        }
    };

    Ok(quote! {
        pub struct #accessor_name<H: ::pult_schema::handle::DataHandle> {
            path: ::pult_schema::path::Path,
            handle: H,
        }

        impl<H: ::pult_schema::handle::DataHandle> ::pult_schema::handle::EntityAccessor for #accessor_name<H> {
            type Value = #entity_name;
            type CreateValue = #create_name;
            type Handle = H;

            fn new(path: ::pult_schema::path::Path, handle: H) -> Self {
                Self { path, handle }
            }
            fn path(&self) -> &::pult_schema::path::Path { &self.path }
            fn handle(&self) -> &H { &self.handle }
        }

        impl<H: ::pult_schema::handle::DataHandle> #accessor_name<H> {
            #(#field_methods)*
            #(#collection_methods)*
        }

        #root_method
    })
}

fn gen_field_accessor_method(field: &FieldMeta) -> syn::Result<TokenStream2> {
    let method_name = &field.ident;
    let ty = &field.ty;
    // The path key is the serde field name. Anything else and the engine writes a
    // field the entity does not have, serde drops it, and the set silently no-ops.
    let path_key_str = field.ident.to_string();

    match field.lifecycle {
        Lifecycle::Local => Ok(quote! {
            pub fn #method_name(&self) -> ::pult_schema::handle::LocalFieldAccessor<#ty, H> {
                ::pult_schema::handle::LocalFieldAccessor::new(
                    ::pult_schema::path::path_key(self.path.clone(), #path_key_str),
                    self.handle.clone(),
                )
            }
        }),
        Lifecycle::Synced => Ok(quote! {
            pub fn #method_name(&self) -> ::pult_schema::handle::FieldAccessor<#ty, H> {
                ::pult_schema::handle::FieldAccessor::new(
                    ::pult_schema::path::path_key(self.path.clone(), #path_key_str),
                    ::pult_schema::lifecycle::Lifecycle::Synced,
                    self.handle.clone(),
                )
            }
        }),
        Lifecycle::Persisted => Ok(quote! {
            pub fn #method_name(&self) -> ::pult_schema::handle::FieldAccessor<#ty, H> {
                ::pult_schema::handle::FieldAccessor::new(
                    ::pult_schema::path::path_key(self.path.clone(), #path_key_str),
                    ::pult_schema::lifecycle::Lifecycle::Persisted,
                    self.handle.clone(),
                )
            }
        }),
    }
}

fn gen_collection_accessor_method(field: &FieldMeta) -> syn::Result<TokenStream2> {
    let method_name = &field.ident;
    let path_key_str = field.ident.to_string();
    let target_type = field.collection_of.as_ref().unwrap();
    let accessor_type = format_ident!("{}Accessor", target_type);

    Ok(quote! {
        pub fn #method_name(&self) -> ::pult_schema::handle::EntityCollectionAccessor<#accessor_type<H>> {
            ::pult_schema::handle::EntityCollectionAccessor::new(
                ::pult_schema::path::path_key(self.path.clone(), #path_key_str),
                self.handle.clone(),
            )
        }
    })
}

// ── PultSqlRow impl ───────────────────────────────────────────────────────────

struct SqlInfo {
    col_def: String,
    bind_expr: TokenStream2,
    read_expr: TokenStream2,
}

fn type_last_ident(ty: &Type) -> Option<String> {
    if let Type::Path(tp) = ty {
        tp.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

fn option_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(tp) = ty {
        let last = tp.path.segments.last()?;
        if last.ident != "Option" {
            return None;
        }
        if let syn::PathArguments::AngleBracketed(args) = &last.arguments {
            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                return Some(inner);
            }
        }
    }
    None
}

fn sql_info_for_field(field_name: &str, ty: &Type, fi: &syn::Ident) -> SqlInfo {
    if let Some(inner) = option_inner_type(ty) {
        let inner_info = sql_info_for_field(field_name, inner, fi);
        let col_def = inner_info.col_def.replace(" NOT NULL", "");
        // A missing, NULL, or unreadable column reads as None rather than panicking.
        // This is the column a newly added optional field has on every existing row,
        // and a panic here would take the process down while opening a show.
        let read_expr = quote! {
            row.get_text(#field_name)
                .filter(|s| !s.is_empty())
                .and_then(|s| ::serde_json::from_str(&s).ok())
        };
        let bind_expr = quote! {
            match &self.#fi {
                Some(v) => ::pult_schema::sql::SqlLiteral::Text(::serde_json::to_string(v).unwrap()),
                None => ::pult_schema::sql::SqlLiteral::Null,
            }
        };
        return SqlInfo { col_def, bind_expr, read_expr };
    }

    let name = type_last_ident(ty).unwrap_or_default();

    match name.as_str() {
        "Uuid" => SqlInfo {
            col_def: format!("{field_name} TEXT NOT NULL"),
            bind_expr: quote! { ::pult_schema::sql::SqlLiteral::Text(self.#fi.to_string()) },
            read_expr: quote! { ::uuid::Uuid::parse_str(&row.get_text(#field_name).unwrap()).unwrap() },
        },
        "String" => SqlInfo {
            col_def: format!("{field_name} TEXT NOT NULL"),
            bind_expr: quote! { ::pult_schema::sql::SqlLiteral::Text(self.#fi.clone()) },
            read_expr: quote! { row.get_text(#field_name).unwrap() },
        },
        "u8" => SqlInfo {
            col_def: format!("{field_name} INTEGER NOT NULL"),
            bind_expr: quote! { ::pult_schema::sql::SqlLiteral::Int(self.#fi as i64) },
            read_expr: quote! { row.get_int(#field_name).unwrap() as u8 },
        },
        "u16" => SqlInfo {
            col_def: format!("{field_name} INTEGER NOT NULL"),
            bind_expr: quote! { ::pult_schema::sql::SqlLiteral::Int(self.#fi as i64) },
            read_expr: quote! { row.get_int(#field_name).unwrap() as u16 },
        },
        "u32" => SqlInfo {
            col_def: format!("{field_name} INTEGER NOT NULL"),
            bind_expr: quote! { ::pult_schema::sql::SqlLiteral::Int(self.#fi as i64) },
            read_expr: quote! { row.get_int(#field_name).unwrap() as u32 },
        },
        "i32" => SqlInfo {
            col_def: format!("{field_name} INTEGER NOT NULL"),
            bind_expr: quote! { ::pult_schema::sql::SqlLiteral::Int(self.#fi as i64) },
            read_expr: quote! { row.get_int(#field_name).unwrap() as i32 },
        },
        "i64" | "u64" => SqlInfo {
            col_def: format!("{field_name} INTEGER NOT NULL"),
            bind_expr: quote! { ::pult_schema::sql::SqlLiteral::Int(self.#fi as i64) },
            read_expr: quote! { row.get_int(#field_name).unwrap() },
        },
        "f32" => SqlInfo {
            col_def: format!("{field_name} REAL NOT NULL"),
            bind_expr: quote! { ::pult_schema::sql::SqlLiteral::Real(self.#fi as f64) },
            read_expr: quote! { row.get_real(#field_name).unwrap() as f32 },
        },
        "f64" => SqlInfo {
            col_def: format!("{field_name} REAL NOT NULL"),
            bind_expr: quote! { ::pult_schema::sql::SqlLiteral::Real(self.#fi) },
            read_expr: quote! { row.get_real(#field_name).unwrap() },
        },
        "bool" => SqlInfo {
            col_def: format!("{field_name} INTEGER NOT NULL"),
            bind_expr: quote! { ::pult_schema::sql::SqlLiteral::Int(self.#fi as i64) },
            read_expr: quote! { row.get_int(#field_name).unwrap() != 0 },
        },
        "DateTime" => SqlInfo {
            col_def: format!("{field_name} TEXT NOT NULL"),
            bind_expr: quote! { ::pult_schema::sql::SqlLiteral::Text(self.#fi.to_rfc3339()) },
            read_expr: quote! { row.get_text(#field_name).unwrap().parse().unwrap() },
        },
        _ => SqlInfo {
            col_def: format!("{field_name} TEXT NOT NULL"),
            bind_expr: quote! {
                ::pult_schema::sql::SqlLiteral::Text(::serde_json::to_string(&self.#fi).unwrap())
            },
            read_expr: quote! {
                ::serde_json::from_str(&row.get_text(#field_name).unwrap()).unwrap()
            },
        },
    }
}

fn gen_pult_sql_row(meta: &StructMeta) -> syn::Result<TokenStream2> {
    let name = &meta.ident;

    let persisted_fields: Vec<&FieldMeta> =
        meta.fields.iter().filter(|f| f.lifecycle == Lifecycle::Persisted).collect();

    if persisted_fields.is_empty() || meta.table_name.is_none() {
        return Ok(quote! {});
    }

    let mut col_def_parts: Vec<String> = Vec::new();
    let mut col_names: Vec<String> = Vec::new();
    let mut bind_exprs: Vec<TokenStream2> = Vec::new();
    let mut read_field_inits: Vec<TokenStream2> = Vec::new();

    for field in &meta.fields {
        let field_name = field.ident.to_string();
        let fi = &field.ident;

        if field.lifecycle == Lifecycle::Persisted {
            let mut info = sql_info_for_field(&field_name, &field.ty, fi);
            if field.is_primary_key {
                info.col_def = format!("{} PRIMARY KEY", info.col_def);
            }
            col_def_parts.push(info.col_def);
            col_names.push(field_name.clone());
            bind_exprs.push(info.bind_expr);
            let read = info.read_expr;
            read_field_inits.push(quote! { #fi: #read });
        } else {
            read_field_inits.push(quote! { #fi: ::std::default::Default::default() });
        }
    }

    let col_defs_str = col_def_parts.join(",\n    ");
    let col_name_strs: Vec<&str> = col_names.iter().map(|s| s.as_str()).collect();

    Ok(quote! {
        impl ::pult_schema::sql::PultSqlRow for #name {
            fn column_defs() -> &'static str {
                #col_defs_str
            }

            fn column_names() -> &'static [&'static str] {
                &[ #(#col_name_strs),* ]
            }

            fn to_sql_values(&self) -> ::std::vec::Vec<::pult_schema::sql::SqlLiteral> {
                vec![ #(#bind_exprs),* ]
            }

            fn from_columns(row: &dyn ::pult_schema::sql::ColumnGetter) -> ::anyhow::Result<Self> {
                Ok(Self {
                    #(#read_field_inits,)*
                })
            }
        }
    })
}

// ── EntityMeta inventory submit ───────────────────────────────────────────────

fn gen_entity_meta_submit(meta: &StructMeta) -> syn::Result<TokenStream2> {
    let name = &meta.ident;
    let entity_name_str = meta.ident.to_string();
    let is_singleton = meta.is_singleton;
    let has_persisted = meta.fields.iter().any(|f| f.lifecycle == Lifecycle::Persisted);
    let has_table = meta.table_name.is_some() && has_persisted;

    let table_name_expr = match &meta.table_name {
        Some(t) => quote! { Some(#t) },
        None => quote! { None },
    };

    let create_fn_name = format_ident!("__pult_create_table_sql_{}", name);
    let column_defs_fn_name = format_ident!("__pult_column_defs_{}", name);
    let lc_fn_name = format_ident!("__pult_field_lifecycles_{}", name);
    let save_fn_name = format_ident!("__pult_save_all_{}", name);
    let load_fn_name = format_ident!("__pult_load_all_{}", name);
    let validate_fn_name = format_ident!("__pult_validate_{}", name);
    let upsert_one_fn_name = format_ident!("__pult_upsert_one_{}", name);
    let delete_one_fn_name = format_ident!("__pult_delete_one_{}", name);

    let pk_expr = match meta.fields.iter().find(|f| f.is_primary_key) {
        Some(f) => { let s = f.ident.to_string(); quote! { Some(#s) } }
        None => quote! { None },
    };

    let (upsert_one_expr, delete_one_expr) = if has_table {
        (quote! { Some(#upsert_one_fn_name) }, quote! { Some(#delete_one_fn_name) })
    } else {
        (quote! { None }, quote! { None })
    };
    let (upsert_one_body, delete_one_body) = if has_table {
        (quote! {
            let entity: #name = ::serde_json::from_value(value)?;
            ::pult_schema::db::upsert(&pool, &entity).await
        }, quote! {
            ::pult_schema::db::delete::<#name>(&pool, id).await
        })
    } else {
        (quote! { Ok(()) }, quote! { let _ = id; Ok(()) })
    };

    let create_fn_body = if has_table {
        quote! { Some(<#name as ::pult_schema::sql::PultSqlRow>::create_table_sql()) }
    } else {
        quote! { None }
    };
    let column_defs_body = if has_table {
        quote! { Some(<#name as ::pult_schema::sql::PultSqlRow>::column_defs()) }
    } else {
        quote! { None }
    };

    // save_all: extract this entity's data from the serialized ShowState snapshot and upsert.
    // load_all: load from SQLite and return in the shape ShowState::Deserialize expects.
    let (save_fn_body, load_fn_body, save_all_expr, load_all_expr) = if has_table {
        let table_str = meta.table_name.as_deref().unwrap();

        let (save_body, load_body) = if meta.is_singleton {
            // Singleton: ShowState field is Option<T>, stored as null|{...} in snapshot.
            (quote! {
                let Some(val) = state_json.get(#table_str) else { return Ok(()); };
                if val.is_null() { return Ok(()); }
                let entity: #name = ::serde_json::from_value(val.clone())?;
                ::pult_schema::db::upsert(&pool, &entity).await
            }, quote! {
                let entities = ::pult_schema::db::get_all::<#name>(&pool).await?;
                Ok(::serde_json::to_value(entities.into_iter().next())?)
            })
        } else {
            // Collection: ShowState field is HashMap<Uuid, T>, stored as {"uuid": {...}} in snapshot.
            (quote! {
                let Some(val) = state_json.get(#table_str) else { return Ok(()); };
                let map: ::std::collections::HashMap<String, #name> =
                    ::serde_json::from_value(val.clone()).unwrap_or_default();
                for entity in map.values() {
                    ::pult_schema::db::upsert(&pool, entity).await?;
                }
                Ok(())
            }, quote! {
                let entities = ::pult_schema::db::get_all::<#name>(&pool).await?;
                let mut map = ::serde_json::Map::new();
                for entity in &entities {
                    let json = ::serde_json::to_value(entity)?;
                    if let Some(id) = json.get("id").and_then(|v| v.as_str()) {
                        map.insert(id.to_string(), json);
                    }
                }
                Ok(::serde_json::Value::Object(map))
            })
        };

        (save_body, load_body,
         quote! { Some(#save_fn_name) },
         quote! { Some(#load_fn_name) })
    } else {
        (quote! { Ok(()) }, quote! { Ok(::serde_json::Value::Null) },
         quote! { None }, quote! { None })
    };

    Ok(quote! {
        #[allow(non_snake_case)]
        fn #create_fn_name() -> Option<String> { #create_fn_body }

        #[allow(non_snake_case)]
        fn #column_defs_fn_name() -> Option<&'static str> { #column_defs_body }

        /// Round-trip through the concrete type so invalid values are rejected
        /// and serde defaults are filled in.
        #[allow(non_snake_case)]
        fn #validate_fn_name(value: ::serde_json::Value) -> ::anyhow::Result<::serde_json::Value> {
            let entity: #name = ::serde_json::from_value(value)?;
            Ok(::serde_json::to_value(entity)?)
        }

        #[allow(non_snake_case)]
        fn #upsert_one_fn_name(
            pool: ::sqlx::SqlitePool,
            value: ::serde_json::Value,
        ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ::anyhow::Result<()>> + Send>> {
            Box::pin(async move { #upsert_one_body })
        }

        #[allow(non_snake_case)]
        fn #delete_one_fn_name(
            pool: ::sqlx::SqlitePool,
            id: ::uuid::Uuid,
        ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ::anyhow::Result<()>> + Send>> {
            let _ = &pool;
            Box::pin(async move { #delete_one_body })
        }

        #[allow(non_snake_case)]
        fn #lc_fn_name() -> &'static [(&'static str, ::pult_schema::lifecycle::Lifecycle)] {
            <#name as ::pult_schema::traits::PultEntity>::field_lifecycles()
        }

        #[allow(non_snake_case)]
        fn #save_fn_name(
            pool: ::sqlx::SqlitePool,
            state_json: ::serde_json::Value,
        ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ::anyhow::Result<()>> + Send>> {
            Box::pin(async move { #save_fn_body })
        }

        #[allow(non_snake_case)]
        fn #load_fn_name(
            pool: ::sqlx::SqlitePool,
        ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ::anyhow::Result<::serde_json::Value>> + Send>> {
            Box::pin(async move { #load_fn_body })
        }

        ::inventory::submit!(::pult_schema::registry::EntityMeta {
            entity_name: #entity_name_str,
            table_name: #table_name_expr,
            is_singleton: #is_singleton,
            create_table_sql: #create_fn_name,
            column_defs: #column_defs_fn_name,
            field_lifecycles: #lc_fn_name,
            save_all: #save_all_expr,
            load_all: #load_all_expr,
            primary_key: #pk_expr,
            validate: #validate_fn_name,
            upsert_one: #upsert_one_expr,
            delete_one: #delete_one_expr,
        });
    })
}

// ── #[pult_commands] implementation ──────────────────────────────────────────

fn pult_commands_impl(impl_block: &mut syn::ItemImpl) -> syn::Result<TokenStream2> {
    let self_ty = &impl_block.self_ty;

    let entity_ident = extract_impl_self_ident(self_ty)
        .ok_or_else(|| syn::Error::new(
            impl_block.self_ty.span(),
            "#[pult_commands] requires a simple type path",
        ))?;
    let entity_name_lower = entity_ident.to_string().to_lowercase();

    let mut generated = TokenStream2::new();

    for item in &mut impl_block.items {
        if let syn::ImplItem::Fn(method) = item {
            let is_command = method.attrs.iter().any(|a| a.path().is_ident("pult_command"));
            if !is_command {
                continue;
            }

            // Extract args_ts from #[pult_command(args = "...")] before stripping the attr.
            let mut args_ts_str = String::new();
            if let Some(attr) = method.attrs.iter().find(|a| a.path().is_ident("pult_command")) {
                let _ = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("args") {
                        let lit: syn::LitStr = meta.value()?.parse()?;
                        args_ts_str = lit.value();
                        Ok(())
                    } else {
                        Err(meta.error("unknown pult_command attribute; expected `args = \"...\"`"))
                    }
                });
            }

            method.attrs.retain(|a| !a.path().is_ident("pult_command"));

            // args_ts stays the single source: the structured form is derived
            // from it here, or left empty where it is too clever to parse.
            let args_schema_str = args_schema_from_ts(&args_ts_str).unwrap_or_default();
            let doc_str = doc_comment_of(&method.attrs);

            let method_name = &method.sig.ident;
            let cmd_name_str = to_camel_case(&method_name.to_string());
            let has_args = method.sig.inputs.len() > 1;

            let call_expr = if has_args {
                quote! { entity.#method_name(args) }
            } else {
                quote! { entity.#method_name() }
            };

            let handler_fn_name =
                format_ident!("__pult_cmd_{}_{}", entity_name_lower, method_name);
            let table_fn_name =
                format_ident!("__pult_cmd_table_{}_{}", entity_name_lower, method_name);

            generated.extend(quote! {
                #[allow(non_snake_case)]
                fn #table_fn_name() -> &'static str {
                    <#self_ty as ::pult_schema::traits::PultEntity>::table_name().unwrap_or("")
                }

                #[allow(non_snake_case)]
                fn #handler_fn_name(
                    entity_json: ::serde_json::Value,
                    args: ::serde_json::Value,
                ) -> ::anyhow::Result<::serde_json::Value> {
                    let mut entity: #self_ty = ::serde_json::from_value(entity_json)?;
                    #call_expr.map_err(|e| ::anyhow::anyhow!("{e}"))?;
                    Ok(::serde_json::to_value(&entity)?)
                }

                ::inventory::submit!(::pult_schema::commands::CommandRegistration {
                    entity_table: #table_fn_name,
                    command_name: #cmd_name_str,
                    is_public: true,
                    args_ts: #args_ts_str,
                    args_schema: #args_schema_str,
                    doc: #doc_str,
                    handler: #handler_fn_name,
                });
            });
        }
    }

    Ok(generated)
}

fn extract_impl_self_ident(ty: &Type) -> Option<&syn::Ident> {
    if let Type::Path(tp) = ty {
        tp.path.segments.last().map(|s| &s.ident)
    } else {
        None
    }
}

/// Derive the structured argument list from an `args = "{ ... }"` TypeScript
/// literal: `"{ cueId: string, at?: number }"` becomes
/// `[{"name":"cueId","type":"string","optional":false},{"name":"at","type":"number","optional":true}]`.
///
/// Only the flat object shape every command so far uses. Anything nested or
/// otherwise clever gets `None` — the TS literal is still pasted verbatim into
/// the frontend types, it just carries no structure for a command line.
fn args_schema_from_ts(args_ts: &str) -> Option<String> {
    let trimmed = args_ts.trim();
    if trimmed.is_empty() {
        return Some("[]".to_string());
    }
    let body = trimmed.strip_prefix('{')?.strip_suffix('}')?;
    if body.contains(['{', '}', '<', '(', '"', '\'']) {
        return None;
    }
    let mut entries = Vec::new();
    for field in body.split(',') {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        let (name, ty) = field.split_once(':')?;
        let (name, optional) = match name.trim().strip_suffix('?') {
            Some(bare) => (bare.trim(), true),
            None => (name.trim(), false),
        };
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        // The type is not validated: it is TypeScript's word, kept as text.
        entries.push(format!(
            r#"{{"name":{},"type":{},"optional":{}}}"#,
            serde_json_string(name),
            serde_json_string(ty.trim()),
            optional
        ));
    }
    Some(format!("[{}]", entries.join(",")))
}

/// A JSON string literal, by hand: this crate has syn and quote, not serde.
fn serde_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The `///` lines of a method, joined the way rustdoc reads them.
fn doc_comment_of(attrs: &[syn::Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let syn::Meta::NameValue(nv) = &attr.meta {
                if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = &nv.value {
                    lines.push(s.value().trim().to_string());
                }
            }
        }
    }
    lines.join("\n")
}

// ── Utility ───────────────────────────────────────────────────────────────────

fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;
    for ch in s.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::args_schema_from_ts;

    #[test]
    fn args_schema_covers_every_registered_shape() {
        // The three commands that exist today.
        assert_eq!(args_schema_from_ts(""), Some("[]".to_string()));
        assert_eq!(
            args_schema_from_ts("{ at?: number }"),
            Some(r#"[{"name":"at","type":"number","optional":true}]"#.to_string())
        );
        assert_eq!(
            args_schema_from_ts("{ cueId: string, at?: number }"),
            Some(
                r#"[{"name":"cueId","type":"string","optional":false},{"name":"at","type":"number","optional":true}]"#
                    .to_string()
            )
        );
    }

    #[test]
    fn a_clever_literal_yields_no_schema_rather_than_a_wrong_one() {
        assert_eq!(args_schema_from_ts("{ nested: { a: string } }"), None);
        assert_eq!(args_schema_from_ts("{ list: Array<string> }"), None);
        assert_eq!(args_schema_from_ts("string"), None);
    }
}
