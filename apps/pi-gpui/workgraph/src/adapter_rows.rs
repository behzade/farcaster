use crate::contract::{Issue, IssueStatus, SessionLink};

pub(super) fn session_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionLink> {
    Ok(SessionLink {
        session_id: row.get(0)?,
        session_path: row.get(1)?,
        issue_number: row.get(2)?,
        linked_at: row.get(3)?,
    })
}

pub(super) fn issue(row: &rusqlite::Row<'_>, project: &str) -> rusqlite::Result<Issue> {
    let status = row.get::<_, String>(3)?;
    Ok(Issue {
        project: project.to_owned(),
        number: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        status: IssueStatus::parse(&status).ok_or(rusqlite::Error::InvalidQuery)?,
        priority: row.get(4)?,
        version: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}
