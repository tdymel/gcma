//! Saving and loading plans as JSON files.

use std::path::Path;

use crate::domain::error::{Error, Result};
use crate::domain::history::plan::{OLDEST_PLAN_VERSION, PLAN_VERSION, Plan};

pub fn save(plan: &Plan, path: &Path) -> Result<()> {
    fs_err::write(path, serde_json::to_vec_pretty(plan)?)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Plan> {
    let data = std::fs::read(path)
        .map_err(|e| Error::Usage(format!("cannot read plan {}: {e}", path.display())))?;
    let plan: Plan = serde_json::from_slice(&data)?;
    if !(OLDEST_PLAN_VERSION..=PLAN_VERSION).contains(&plan.version) {
        return Err(Error::Usage(format!(
            "unsupported plan version {}",
            plan.version
        )));
    }
    plan.validate()?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::history::plan::{Entry, PIdent, Parent};
    use crate::domain::settings::Signing;

    #[test]
    fn plan_roundtrip_and_forward_reference_rejected() {
        let id = PIdent {
            name: "N".into(),
            email: "e@x".into(),
            time: 1,
            tz: 60,
        };
        let e = Entry {
            old_oid: "a".repeat(40),
            parents: vec![Parent::Base("b".repeat(40))],
            author: id.clone(),
            committer: id,
            message: b"hi \xff\n".to_vec(),
            tree: None,
            gitignore: false,
        };
        let mut p = Plan {
            version: PLAN_VERSION,
            branch_ref: "refs/heads/main".into(),
            tip_oid: "a".repeat(40),
            signing: Signing::Strip,
            entries: vec![e.clone()],
            paths: None,
            dropped: Vec::new(),
            new_tip: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.json");
        save(&p, &path).unwrap();
        let q = load(&path).unwrap();
        assert_eq!(q.entries[0].message, b"hi \xff\n");
        assert_eq!(q.branch_name(), "main");
        p.entries[0].parents = vec![Parent::In(0)];
        save(&p, &path).unwrap();
        assert!(load(&path).is_err());
        // Ids and the branch ref are validated too.
        p.entries[0].parents = vec![Parent::Base("--output=x".into())];
        save(&p, &path).unwrap();
        assert!(load(&path).is_err());
        p.entries[0].parents = vec![];
        p.branch_ref = "refs/tags/v1".into();
        save(&p, &path).unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn utf8_text_is_a_plain_string_and_other_bytes_are_base64() {
        use crate::domain::history::wire::Text;
        let plain = serde_json::to_string(&Text::from("Jörg")).unwrap();
        assert_eq!(plain, "\"Jörg\"");
        let raw = Text::from_bytes(b"J\xf6rg");
        let json = serde_json::to_string(&raw).unwrap();
        assert_eq!(json, "{\"base64\":\"SvZyZw==\"}");
        assert_eq!(serde_json::from_str::<Text>(&json).unwrap(), raw);
        assert_eq!(
            serde_json::from_str::<Text>(&plain).unwrap().as_str(),
            Some("Jörg")
        );
        assert!(serde_json::from_str::<Text>("{\"base64\":\"%%\"}").is_err());
    }
}
