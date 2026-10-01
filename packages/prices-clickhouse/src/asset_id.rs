//! The one asset-id expression (task 0139).
//!
//! An asset's id is a function of its identity, computed by ClickHouse:
//!
//! ```text
//! xxh3(concat(asset_code, ':', issuer_address, ':', contract_address))   -- UInt64
//! ```
//!
//! - Case is preserved: `usdc` and `USDC` under one issuer are two assets.
//! - An absent field is the empty string, so native XLM is `XLM::`.
//! - 0 is never an asset's id: `oracle_prices` uses it as the "no asset"
//!   sentinel (`ORACLE_FEED_NO_ASSET_ID`). The tables that hold ids carry a
//!   CHECK that refuses 0 and `xxh3('::')`, the id of a blank identity.
//!
//! The database computes the id, never the backend. This module only renders
//! SQL text; nothing here hashes in Rust. A test that needs a numeric id asks
//! the server for it (`fixture::fetch_id`).
//!
//! Every schema statement, migration query and test fixture builds the id from
//! these functions, so there is exactly one place the formula is written.

/// The id of an identity held in three SQL expressions (usually column names).
///
/// `id_expr("asset_code", "issuer_address", "contract_address")` renders
/// `xxh3(concat(asset_code, ':', issuer_address, ':', contract_address))`.
pub fn id_expr(code_sql: &str, issuer_sql: &str, contract_sql: &str) -> String {
    format!(
        "xxh3({})",
        identity_concat(code_sql, issuer_sql, contract_sql)
    )
}

/// [`id_expr`] for `oracle_prices`, where a blank identity is the REDSTONE
/// sentinel 0 rather than an error.
pub fn oracle_id_expr(code_sql: &str, issuer_sql: &str, contract_sql: &str) -> String {
    format!(
        "if({} = '::', 0, {})",
        identity_concat(code_sql, issuer_sql, contract_sql),
        id_expr(code_sql, issuer_sql, contract_sql)
    )
}

/// The id of a literal identity, e.g. `id_of("XLM", "", "")`.
pub fn id_of(code: &str, issuer: &str, contract: &str) -> String {
    id_expr(&sql_str(code), &sql_str(issuer), &sql_str(contract))
}

/// A ClickHouse string literal: single-quoted, with `\` and `'` escaped.
pub fn sql_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\\' || c == '\'' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
    out
}

fn identity_concat(code_sql: &str, issuer_sql: &str, contract_sql: &str) -> String {
    format!("concat({code_sql}, ':', {issuer_sql}, ':', {contract_sql})")
}

/// Test fixtures that need asset ids. Not part of the API.
///
/// Today's schema still stores `asset_id` as `UInt32`, so a fixture id is the
/// derived id truncated to 32 bits (`toUInt32(xxh3(...))`). Fixtures written
/// against these helpers keep working when the schema changes: the commit that
/// moves the schema to `UInt64` switches `id` and `assets_insert` to the
/// `UInt64` form in the same change.
#[doc(hidden)]
pub mod fixture {
    use super::{id_of, sql_str};

    /// One `prices.assets` row for `assets_insert`.
    ///
    /// Displays as its fixture id, so a format string names the asset's id as
    /// `{FOO}` and the row and its candles cannot disagree.
    #[derive(Debug, Clone, Copy)]
    pub struct AssetFixture<'a> {
        pub code: &'a str,
        pub asset_type: &'a str,
        pub issuer: &'a str,
        pub contract: &'a str,
        pub sac: &'a str,
    }

    impl<'a> AssetFixture<'a> {
        /// A row with no SAC.
        pub const fn new(
            code: &'a str,
            asset_type: &'a str,
            issuer: &'a str,
            contract: &'a str,
        ) -> Self {
            Self {
                code,
                asset_type,
                issuer,
                contract,
                sac: "",
            }
        }

        /// The same row, wrapped by the SAC `sac`.
        pub const fn with_sac(self, sac: &'a str) -> Self {
            Self { sac, ..self }
        }

        /// This row's fixture id, as a SQL expression.
        pub fn id(&self) -> String {
            id(self.code, self.issuer, self.contract)
        }
    }

    impl std::fmt::Display for AssetFixture<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.id())
        }
    }

    /// The fixture id of an identity, as a SQL expression.
    pub fn id(code: &str, issuer: &str, contract: &str) -> String {
        format!("toUInt32({})", id_of(code, issuer, contract))
    }

    /// `INSERT INTO <db>.assets` for `rows`, with every id derived from its
    /// row's identity.
    pub fn assets_insert(db: &str, rows: &[AssetFixture<'_>]) -> String {
        let values: Vec<String> = rows
            .iter()
            .map(|r| {
                format!(
                    "({}, {}, {}, {}, {}, {})",
                    id(r.code, r.issuer, r.contract),
                    sql_str(r.code),
                    sql_str(r.asset_type),
                    sql_str(r.issuer),
                    sql_str(r.contract),
                    sql_str(r.sac),
                )
            })
            .collect();
        format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) \
             VALUES {}",
            values.join(", ")
        )
    }

    /// The fixture id of an identity, computed by the server.
    ///
    /// # Panics
    ///
    /// If the query fails. This is a test helper.
    pub async fn fetch_id(
        client: &clickhouse::Client,
        code: &str,
        issuer: &str,
        contract: &str,
    ) -> u64 {
        let sql = format!("SELECT toUInt64({})", id(code, issuer, contract));
        client
            .query(&sql)
            .fetch_one::<u64>()
            .await
            .unwrap_or_else(|e| panic!("fetch_id({code:?}, {issuer:?}, {contract:?}): {e}"))
    }

    /// The fixture ids of `rows`, in order, computed by the server.
    ///
    /// # Panics
    ///
    /// If the query fails. This is a test helper.
    pub async fn fetch_ids<const N: usize>(
        client: &clickhouse::Client,
        rows: &[AssetFixture<'_>; N],
    ) -> [u64; N] {
        let ids: Vec<String> = rows
            .iter()
            .map(|r| format!("toUInt64({})", r.id()))
            .collect();
        let sql = format!("SELECT [{}]", ids.join(", "));
        let got: Vec<u64> = client
            .query(&sql)
            .fetch_one()
            .await
            .unwrap_or_else(|e| panic!("fetch_ids: {e}"));
        got.try_into()
            .unwrap_or_else(|v: Vec<u64>| panic!("fetch_ids: {} ids for {N} rows", v.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{AssetFixture, assets_insert, id};
    use super::*;

    #[test]
    fn id_expr_is_the_d1_formula() {
        assert_eq!(
            id_expr("asset_code", "issuer_address", "contract_address"),
            "xxh3(concat(asset_code, ':', issuer_address, ':', contract_address))"
        );
    }

    #[test]
    fn oracle_id_expr_maps_a_blank_identity_to_zero() {
        assert_eq!(
            oracle_id_expr("asset_code", "issuer_address", "contract_address"),
            "if(concat(asset_code, ':', issuer_address, ':', contract_address) = '::', 0, \
             xxh3(concat(asset_code, ':', issuer_address, ':', contract_address)))"
        );
    }

    #[test]
    fn id_of_quotes_each_field() {
        assert_eq!(
            id_of("XLM", "", ""),
            "xxh3(concat('XLM', ':', '', ':', ''))"
        );
        assert_eq!(
            id_of("USDC", "GA5Z", "CAS3"),
            "xxh3(concat('USDC', ':', 'GA5Z', ':', 'CAS3'))"
        );
    }

    #[test]
    fn sql_str_escapes_quote_and_backslash() {
        let cases = [
            ("", "''"),
            ("XLM", "'XLM'"),
            ("O'B", r"'O\'B'"),
            (r"a\b", r"'a\\b'"),
            (r"\'", r"'\\\''"),
            ("A\u{FFFD}B", "'A\u{FFFD}B'"),
        ];
        for (input, want) in cases {
            assert_eq!(sql_str(input), want, "sql_str({input:?})");
        }
    }

    #[test]
    fn fixture_id_is_the_uint32_form() {
        assert_eq!(
            id("XLM", "", ""),
            "toUInt32(xxh3(concat('XLM', ':', '', ':', '')))"
        );
    }

    #[test]
    fn fixture_constructors_fill_the_row() {
        let xlm = AssetFixture::new("XLM", "native", "", "").with_sac("CXLMSAC");
        assert_eq!(
            (xlm.code, xlm.asset_type, xlm.issuer, xlm.contract, xlm.sac),
            ("XLM", "native", "", "", "CXLMSAC")
        );
        assert_eq!(AssetFixture::new("USDC", "classic", "GA5Z", "").sac, "");
        let token = AssetFixture::new("", "contract", "", "CTOK");
        assert_eq!(token.id(), id("", "", "CTOK"));
        assert_eq!(format!("({token}, 1)"), format!("({}, 1)", token.id()));
    }

    #[test]
    fn fixture_assets_insert_names_the_derived_id() {
        let rows = [
            AssetFixture {
                code: "XLM",
                asset_type: "native",
                issuer: "",
                contract: "",
                sac: "CXLMSAC",
            },
            AssetFixture {
                code: "USDC",
                asset_type: "classic",
                issuer: "GA5Z",
                contract: "",
                sac: "",
            },
        ];
        assert_eq!(
            assets_insert("it_db", &rows),
            "INSERT INTO it_db.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) \
             VALUES (toUInt32(xxh3(concat('XLM', ':', '', ':', ''))), 'XLM', 'native', '', '', 'CXLMSAC'), \
             (toUInt32(xxh3(concat('USDC', ':', 'GA5Z', ':', ''))), 'USDC', 'classic', 'GA5Z', '', '')"
        );
    }
}
