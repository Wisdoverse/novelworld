use std::ffi::OsStr;

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct ProfileIdentity {
    contract: String,
    profile: String,
}

#[derive(Serialize)]
struct Capability<'a> {
    contract: &'a str,
    profile: &'a str,
    profile_sha256: String,
}

pub fn capability_output() -> String {
    let identity: ProfileIdentity =
        serde_json::from_str(crate::diagnostic_budget::PROFILE_JSON).expect("valid profile JSON");
    serde_json::to_string(&Capability {
        contract: &identity.contract,
        profile: &identity.profile,
        profile_sha256: crate::diagnostic_budget::profile_sha256(),
    })
    .expect("capability JSON is serializable")
}

pub fn capability_probe<I, S>(args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut args = args.into_iter();
    let flag = args.next()?;
    if args.next().is_some() || flag.as_ref() != OsStr::new("--diagnostic-budget-contract") {
        return None;
    }
    Some(capability_output())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_probe_is_exact_and_content_free() {
        assert_eq!(capability_probe(std::iter::empty::<&str>()), None);
        let output = capability_probe(["--diagnostic-budget-contract"]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["contract"], "llm-diagnostic-budget-v1");
        assert_eq!(value["profile"], "vision-journey-diagnostic-v1");
        assert_eq!(
            value["profile_sha256"],
            "a589b4cb0e4968f5624f8d4039c262ab330ecd5f9524204b39cfa868c8257839"
        );
        assert_eq!(capability_probe(["--unknown"]), None);
        assert_eq!(
            capability_probe(["--diagnostic-budget-contract", "extra"]),
            None
        );
    }
}
