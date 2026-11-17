use super::*;
use agents::accounts::AccountsSnapshot;

impl StateStore {
    pub fn load_accounts(&self) -> Result<AccountsSnapshot, String> {
        let json: Option<String> = self
            .connection
            .query_row("SELECT body FROM account_state WHERE id=1", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| format!("Read accounts: {e}"))?;
        json.map(|json| serde_json::from_str(&json).map_err(|e| format!("Decode accounts: {e}")))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub fn save_accounts(&mut self, snapshot: &AccountsSnapshot) -> Result<(), String> {
        let json = serde_json::to_string(snapshot).map_err(|e| format!("Encode accounts: {e}"))?;
        self.connection.execute("INSERT INTO account_state(id, body) VALUES(1, ?1) ON CONFLICT(id) DO UPDATE SET body=excluded.body", [json])
            .map(|_| ()).map_err(|e| format!("Save accounts: {e}"))
    }
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
