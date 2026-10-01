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
//!   sentinel. The tables that hold ids carry a CHECK ([`derived_id_check`])
//!   that refuses 0 and `xxh3('::')`, the id of a blank identity.
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

/// The body of a `CHECK` that refuses, in each of `id_cols`, the two values no
/// asset's id may take: 0 and the id of a blank identity. It has to name the
/// stored ids: ClickHouse refuses a CHECK on EPHEMERAL columns.
pub fn derived_id_check(id_cols: &[&str]) -> String {
    let blank = id_of("", "", "");
    id_cols
        .iter()
        .map(|c| format!("{c} != 0 AND {c} != {blank}"))
        .collect::<Vec<_>>()
        .join(" AND ")
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
/// A fixture id is the schema's own expression over a literal identity, and
/// `assets_insert` leaves `asset_id` to the table (naming a MATERIALIZED column
/// is refused), so a fixture's ids are the ones the database derives.
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
        id_of(code, issuer, contract)
    }

    /// `INSERT INTO <db>.assets` for `rows`. The table derives each id.
    pub fn assets_insert(db: &str, rows: &[AssetFixture<'_>]) -> String {
        let values: Vec<String> = rows
            .iter()
            .map(|r| {
                format!(
                    "({}, {}, {}, {}, {})",
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
             (asset_code, asset_type, issuer_address, contract_address, sac_address) \
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
    use crate::INIT_SQL;

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
    fn derived_id_check_refuses_zero_and_the_blank_identity_per_column() {
        assert_eq!(
            derived_id_check(&["asset_id", "quote_asset_id"]),
            "asset_id != 0 AND asset_id != xxh3(concat('', ':', '', ':', '')) AND \
             quote_asset_id != 0 AND quote_asset_id != xxh3(concat('', ':', '', ':', ''))"
        );
    }

    /// init.sql is a static file, so it cannot call this module. It holds this
    /// module's renderings verbatim instead, and this test keeps them equal.
    #[test]
    fn init_sql_spells_every_id_as_this_module_renders_it() {
        let identity = id_expr("asset_code", "issuer_address", "contract_address");
        let wanted = [
            format!("asset_id         UInt64        MATERIALIZED {identity},"),
            format!(
                "CONSTRAINT asset_id_derived CHECK {}\n",
                derived_id_check(&["asset_id"])
            ),
            format!(
                "asset_id         UInt64        DEFAULT {},",
                id_expr("base_code", "base_issuer", "base_contract")
            ),
            format!(
                "quote_asset_id   UInt64        DEFAULT {},",
                id_expr("quote_code", "quote_issuer", "quote_contract")
            ),
            format!(
                "CONSTRAINT asset_ids_derived CHECK {}\n",
                derived_id_check(&["asset_id", "quote_asset_id"])
            ),
            format!(
                "asset_id      UInt64        DEFAULT {},",
                oracle_id_expr("asset_code", "issuer_address", "contract_address")
            ),
        ];
        for want in &wanted {
            assert_eq!(INIT_SQL.matches(want.as_str()).count(), 1, "{want}");
        }
        // Every id column is one of the above or a plain UInt64 copy.
        for line in INIT_SQL.lines().map(str::trim) {
            if line.starts_with("asset_id ") || line.starts_with("quote_asset_id ") {
                assert!(
                    line.split_whitespace()
                        .nth(1)
                        .map(|t| t.trim_end_matches(','))
                        == Some("UInt64"),
                    "id column not UInt64: {line}"
                );
            }
        }
    }

    #[test]
    fn fixture_id_is_the_schema_expression() {
        assert_eq!(id("XLM", "", ""), "xxh3(concat('XLM', ':', '', ':', ''))");
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
    fn fixture_assets_insert_leaves_the_id_to_the_table() {
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
             (asset_code, asset_type, issuer_address, contract_address, sac_address) \
             VALUES ('XLM', 'native', '', '', 'CXLMSAC'), ('USDC', 'classic', 'GA5Z', '', '')"
        );
    }
}
