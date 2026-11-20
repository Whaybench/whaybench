//! How a turn's prompt is shown: what the agent was sent is kept as it was, but a line meant for a person shows what
//! a person would call it. Claude Code sends a finished background task and a subagent's report as prompts, and
//! wraps pasted text; Whaybench's own requests start with a bracketed name.

/// The text between `<tag ...>` and `</tag>` when `p` starts with exactly that tag and closes it; None otherwise.
fn inside<'a>(p: &'a str, tag: &str) -> Option<&'a str> {
    let rest = p.trim_start().strip_prefix(&format!("<{tag}"))?;
    if !rest.starts_with(['>', ' ', '\n', '\t']) { return None; }
    let rest = &rest[rest.find('>')? + 1..];
    rest.find(&format!("</{tag}>")).map(|end| &rest[..end])
}

/// Pasted text without Claude Code's wrapping (it always carries an id), with whatever the person wrote around it.
fn unwrapped(p: &str) -> Option<String> {
    let start = p.find("<pasted_content id=")?;
    let open_end = start + p[start..].find('>')? + 1;
    let (inner, after) = p[open_end..].split_once("</pasted_content>")?;
    Some(format!("{} {} {}", &p[..start], inner, after).split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The answer panel's line for a prompt: a task's summary, a subagent's report, pasted text, the name of a
/// request of Whaybench's. Anything else, a person's own '[Bug] ...' too, stays as it is.
pub fn label(p: &str) -> String {
    if let Some(n) = inside(p, "task-notification") {
        let tag = |t: &str| n.find(&format!("<{t}>")).and_then(|i| inside(&n[i..], t)).unwrap_or("").trim();
        // The task by its name, the quoted part of 'Background command "build" completed (exit code 0)', and how
        // it ended; the exit code and the rest are for the agent.
        let summary = tag("summary");
        let name = summary.split('"').nth(1).filter(|q| !q.trim().is_empty()).unwrap_or(summary).trim();
        let ended = match tag("status") { "completed" => crate::i18n::t("prompt-task-done"), "failed" => crate::i18n::t("prompt-task-failed"), "" => "", _ => crate::i18n::t("prompt-task-stopped") };
        let line = [name, ended].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
        return if line.is_empty() { crate::i18n::t("prompt-task-bare").to_string() } else { crate::i18n::f("prompt-task", &[("a1", line)]) };
    }
    if inside(p, "agent-message").is_some() { return crate::i18n::t("prompt-subagent").to_string(); }
    if let Some(text) = unwrapped(p) { return text; }
    if crate::i18n::request_heads().iter().any(|h| p.starts_with(h)) && let Some(end) = p.find(']') { return p[..=end].to_string(); }
    p.to_string()
}

/// What the person asked, for the session list; None for a command, a request of Whaybench's, a task's notice or a
/// subagent's report.
pub fn asked(p: &str) -> Option<String> {
    let t = p.trim_start();
    if t.starts_with('/') || t.starts_with('[') || inside(t, "task-notification").is_some() || inside(t, "agent-message").is_some() { return None; }
    Some(unwrapped(p).unwrap_or_else(|| p.split_whitespace().collect::<Vec<_>>().join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_show_what_a_person_would_call_them() {
        let notice = "<task-notification>\n<task-id>b1</task-id>\n<status>completed</status>\n<summary>Background command \"build\" completed (exit code 0)</summary>\n</task-notification>";
        assert_eq!(label(notice), crate::i18n::f("prompt-task", &[("a1", format!("build · {}", crate::i18n::t("prompt-task-done")))]));
        let failed = "<task-notification><status>failed</status><summary>Background command \"test\" failed with exit code 1</summary></task-notification>";
        assert!(label(failed).contains(&format!("test · {}", crate::i18n::t("prompt-task-failed"))), "{}", label(failed));
        assert!(!label(failed).contains("exit code"));
        assert_eq!(asked(notice), None);
        let report = "<agent-message from=\"a1\">\n[Subagent hand-back] ...\n</agent-message>";
        assert_eq!(label(report), crate::i18n::t("prompt-subagent"));
        assert_eq!(asked(report), None);
        let pasted = "see this <pasted_content id=\"1da3\">\nline one\nline two</pasted_content> please";
        assert_eq!(label(pasted), "see this line one line two please");
        assert_eq!(asked(pasted).as_deref(), Some("see this line one line two please"));
        assert_eq!(label("[Map update] Bring this session's map up to date"), "[Map update]");
        assert_eq!(asked("[Map update] x"), None);
        assert_eq!(asked("/compact"), None);
        assert_eq!(label("plain  question"), "plain  question");
        assert_eq!(asked("plain \n question").as_deref(), Some("plain question"));
        // Only the exact, closed tags: a person writing about them keeps their words.
        assert_eq!(label("<agent-message-example> what is this?"), "<agent-message-example> what is this?");
        assert_eq!(asked("<agent-message> unclosed").as_deref(), Some("<agent-message> unclosed"));
        assert_eq!(label("What does <pasted_content> mean?"), "What does <pasted_content> mean?");
        assert_eq!(label("[Bug] Why does saving fail?"), "[Bug] Why does saving fail?");
        assert_eq!(label("[Project work 3] First run"), "[Project work 3]");
        assert_eq!(label("<task-notification><task-id>x</task-id></task-notification>"), crate::i18n::t("prompt-task-bare"));
    }
}
