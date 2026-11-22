//! The project command. The AI writes projects only through this.
use std::{io::Read, path::Path};

use serde_json::{Value, json};
use wb_core::paths::{self, Data};
use wb_store::{Res, Store, folder, markdown::headings, s};

const HELP: &str = r#"Project store command. Only the AI writes projects, and only through this; the person asks.

  whaybench-project list | tree                    projects (tree: parent by id)
  whaybench-project new ID NAME [--parent P] [--path DIR] [--always] [--desc D] [--folder DIR [--from SRC] [--encrypt]]
                                                   --folder makes a Briefsafe project: its folder (a btrfs subvolume
                                                   where it can; --from copies a folder in), its data inside it
                                                   (<folder>/.whaybench), and the first snapshot. Its documents and map
                                                   are worked on with WHAYBENCH_DATA_DIR=<folder>/.whaybench
  whaybench-project open ID [--recovery] | close ID             let the host's programs see a Briefsafe project's data, or stop
  whaybench-project snapshot ID | snapshots ID     take a snapshot of the project folder (30 kept) / list them
  whaybench-project restore ID SNAPSHOT            put the folder back as it was; the state now is kept as a snapshot.
                                                   Run it from outside the project folder, with nothing working in it
  whaybench-project forget ID                      take a closed Briefsafe project off the list; its folder stays
  whaybench-project move ID PARENT|-               put a project under another (- = top)
  whaybench-project for-cwd [DIR]                  project linked to a folder
  whaybench-project map ID                         the project map (JSON)
  whaybench-project map-put ID [FILE|-]            write the project map: {"areas": [{"id", "title", "note"?, "order"?}],
                                                   "items": [{"id", "area", "parent"?, "text", "note"?, "stage":
                                                   idea|decided|doing|unverified|done|resolved|dropped, "include" (product scope,
                                                   not implementation): include|hold|exclude, "reason", "origin": [{"session", "node", "label"}]}],
                                                   "remove": [ids]}. Items are your own words, readable without the
                                                   session they came from; never a copy of node text.
  whaybench-project show ID DOC                    a document of the project (Markdown)
  whaybench-project outline ID DOC                 its headings with line numbers
  whaybench-project checkout ID DOC                prints the path of this session's working copy; edit it with your
                                                   normal file tools
  whaybench-project put ID DOC --what "one line on what changed" the working copy becomes the document; fails if the document changed
                                                   since checkout
  whaybench-project recent ID [N]                  changes by place
  whaybench-project archive ID DOC                 take a finished document out of the tabs; it stays whole and readable
  whaybench-project change-items ID EVENT          before/after of each map item a recorded change touched
  whaybench-project change ID DOC EVENT            before/after of each section a recorded change touched
  whaybench-project take ID WORK                   take a work item the program sent ([Project work WORK]): prints
                                                   taken (go on) or already (done or taken elsewhere: do nothing)
  whaybench-project done ID WORK [--note N]        the work item is finished
  whaybench-project queue ID                       work items (waiting / taken / done)

A project is its map; documents are templates the person adds. Write only the documents it has, and only when asked;
follow the <!-- guide: --> comments in them. Do not create documents.
The session id comes from --session, else WHAYBENCH_SESSION_ID, else the id Claude Code sets."#;

fn out(v: Value) {
    match v {
        Value::String(t) => println!("{t}"),
        other => println!("{}", serde_json::to_string_pretty(&other).unwrap_or_default()),
    }
}

fn opt(rest: &mut Vec<String>, name: &str) -> Option<String> {
    let i = rest.iter().position(|x| x == name)?;
    if i + 1 >= rest.len() { rest.remove(i); return None; }
    let v = rest.remove(i + 1);
    rest.remove(i);
    Some(v)
}

fn flag(rest: &mut Vec<String>, name: &str) -> bool {
    let had = rest.iter().any(|x| x == name);
    rest.retain(|x| x != name);
    had
}

fn calling_session() -> String {
    ["WHAYBENCH_SESSION_ID", "CLAUDE_CODE_SESSION_ID", "CLAUDE_SESSION_ID"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())).unwrap_or_default()
}

fn absolute(p: &str) -> String {
    let path = Path::new(p);
    let joined = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    joined.components().as_path().to_string_lossy().trim_end_matches('/').to_string()
}

fn input(rest: &[String]) -> Res<Value> {
    let source = rest.get(1).map(String::as_str).unwrap_or("-");
    let mut t = String::new();
    if source == "-" { std::io::stdin().read_to_string(&mut t).map_err(|e| e.to_string())?; } else { t = std::fs::read_to_string(source).map_err(|e| e.to_string())?; }
    serde_json::from_str(&t).map_err(|e| e.to_string())
}

fn stub_of(st: &Store, pid: &str) -> Res<wb_core::roots::Stub> {
    wb_core::roots::stubs(&st.data).into_iter().find(|s| s.id == pid)
        .ok_or_else(|| format!("no project with its own folder: {pid} (made with new --folder)"))
}

/// The stub, and its folder still this project's (another project may have been made where it was).
fn owned(st: &Store, pid: &str) -> Res<wb_core::roots::Stub> {
    let s = stub_of(st, pid)?;
    if !s.intact() { return Err(format!("{} does not hold project {pid}'s data now", s.folder.display())); }
    Ok(s)
}

fn arg(rest: &[String], i: usize) -> Res<&str> { rest.get(i).map(String::as_str).ok_or_else(|| "missing argument".to_string()) }

fn run(a: Vec<String>, st: &Store) -> Res<i32> {
    let Some(cmd) = a.first().cloned() else { println!("{HELP}"); return Ok(0) };
    if cmd == "-h" || cmd == "--help" { println!("{HELP}"); return Ok(0); }
    let mut rest: Vec<String> = a[1..].to_vec();
    let session = opt(&mut rest, "--session").unwrap_or_else(calling_session);
    match cmd.as_str() {
        "list" => {
            let mut all: Vec<Value> = st.list_projects().iter().map(|p| json!({"id": p["id"], "name": p["name"], "parent": p.get("parent"), "docs": p["docs"], "paths": p.get("paths")})).collect();
            // A Briefsafe project's content is in its own folder: the host lists its name and folder only.
            all.extend(wb_core::roots::stubs(&st.data).into_iter().map(|s| json!({"id": s.id, "name": s.name, "kind": "briefsafe", "folder": s.folder})));
            out(Value::Array(all));
        }
        "tree" => {
            let t: Vec<String> = st.tree().iter().map(|(p, d)| format!("{}{} [{}]", "  ".repeat(*d), s(p, "name"), s(p, "id"))).collect();
            out(json!(if t.is_empty() { "(none)".to_string() } else { t.join("\n") }));
        }
        "new" => {
            let parent = opt(&mut rest, "--parent").unwrap_or_default();
            let desc = opt(&mut rest, "--desc").unwrap_or_default();
            let mut paths = Vec::new();
            while let Some(p) = opt(&mut rest, "--path") { paths.push(absolute(&p)); }
            let always = flag(&mut rest, "--always");
            let folder = opt(&mut rest, "--folder").map(|f| absolute(&f));
            let from = opt(&mut rest, "--from").map(|f| absolute(&f));
            let encrypt = flag(&mut rest, "--encrypt");
            let (pid, name) = (arg(&rest, 0)?, arg(&rest, 1)?);
            let Some(folder) = folder else {
                if from.is_some() { return Err("--from needs --folder".into()); }
                // One id for one project: a Briefsafe project's id is taken too.
                if wb_core::roots::stubs(&st.data).iter().any(|s| s.id == pid) { return Err(format!("project exists: {pid}")); }
                out(st.create_project(pid, name, &desc, &parent, paths, always)?);
                return Ok(0);
            };
            // One creation at a time: two with the same id must not both pass the check, nor one undo the other's.
            // A vault only where the project runs in a container: its key must never reach a host-run agent (spec 3.22).
            if encrypt && !wb_core::container::isolation() { return Err("--encrypt needs podman and a container image for this system".into()); }
            // In turn with an open of the same id too: it must not find the project half made.
            let would_be = wb_core::roots::Stub::new(&st.data, pid, name, Path::new(&folder));
            let (p, first, recovery) = wb_core::container::life(&would_be, || wb_core::container::flocked(&st.data.cache().join("project-create.lock"), || st.create_in_folder(pid, name, &desc, &parent, paths, always, &folder, from.as_deref().map(Path::new), encrypt)))?;
            // The recovery key, once: it opens the vault when the keyring has lost its key. Kept by the person, nowhere here.
            let note = recovery.as_ref().map(|_| "Keep this recovery key somewhere safe, apart from this computer: it is shown only now.");
            out(json!({"project": p, "snapshot": first.as_ref().map(|t| &t.name).filter(|n| !n.is_empty()), "warning": first.and_then(|t| t.warning),
                "recovery_key": recovery, "note": note}));
        }
        "open" => {
            let recovery = flag(&mut rest, "--recovery");
            let st2 = stub_of(st, arg(&rest, 0)?)?;
            // Opened in its container before: there again, or not at all.
            let isolated = wb_core::container::opened_isolated(&st2) || wb_core::container::isolation();
            if st2.vault && !isolated { return Err("this project's vault opens only in its container: podman and an image are needed".into()); }
            wb_core::container::life(&st2, || {
            // The vault first: with the keyring's key, or with the recovery key on standard input.
            if st2.vault {
                if recovery {
                    let mut key = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut key).map_err(|e| e.to_string())?;
                    wb_core::vault::mount_with_recovery(&st2, &key)?;
                } else {
                    wb_core::vault::mount(&st2).map_err(|e| if e == wb_core::vault::NO_KEY { format!("{e}: open it with its recovery key, `whaybench-project open {} --recovery` (the key on standard input)", st2.id) } else { e })?;
                }
            }
            let opened = (|| -> Res<()> {
            // Isolated where a container can run: the container first, the open place only once it runs (spec 3.22).
            if isolated {
                let own = wb_core::paths::own_exe().map_err(|e| e.to_string())?;
                wb_core::container::start(&st2, &wb_core::container::programs(&own))?;
            }
            if !isolated {
                // Without isolation its agents still keep their own Claude Code folder (spec 3.22).
                let home = std::env::var_os("HOME").map(std::path::PathBuf::from).ok_or("no home folder")?;
                wb_core::container::prepare_claude(&st2, &home)?;
            }
            // Noted before the open place is there: nothing ever sees it open without knowing it is isolated.
            if isolated && let Err(e) = wb_core::container::note_isolated(&st2, true) {
                let _ = wb_core::container::stop(&st2);
                return Err(e.to_string());
            }
            if let Err(e) = wb_core::roots::open_plain(&st2) {
                if isolated { let _ = wb_core::container::stop(&st2); let _ = wb_core::container::note_isolated(&st2, false); }
                return Err(e.to_string());
            }
            wb_core::container::tidy(&st2, isolated);
            Ok(())
            })();
            // Not opened: nothing of it is left open (a link from an earlier open whose container died, the vault).
            if opened.is_err() && st2.vault {
                let _ = wb_core::container::stop(&st2);
                if wb_core::vault::unmount(&st2).is_ok() { let _ = wb_core::roots::close_plain(&st2); let _ = wb_core::container::note_isolated(&st2, false); }
            }
            opened
            })?;
            out(json!({"open": st2.id, "data": st2.open_at, "isolated": isolated}));
        }
        "close" => {
            let st2 = stub_of(st, arg(&rest, 0)?)?;
            // The open place goes only once the container is gone: a project is never taken for closed while it runs.
            wb_core::container::life(&st2, || {
                if wb_core::container::podman_found() { wb_core::container::stop(&st2)?; }
                // The vault before the open place: one that will not close (a panel holds a file in it) leaves the
                // project open, said so, and never closed in name with its data still in the clear.
                if st2.vault { wb_core::vault::unmount(&st2).map_err(|e| format!("{e}; the project stays open: close what uses it, then close again"))?; }
                wb_core::roots::close_plain(&st2).map_err(|e| e.to_string())?;
                wb_core::container::note_isolated(&st2, false).map_err(|e| e.to_string())?;
                Ok(())
            })?;
            out(json!({"closed": st2.id}));
        }
        "snapshot" => {
            // Its container held still meanwhile: one moment of every file, not files caught half written.
            let stub = owned(st, arg(&rest, 0)?)?;
            // In turn with open, close and restore: a restore must not lose the snapshot it goes back to.
            let t = wb_core::container::life(&stub, || wb_core::container::paused(&stub, || folder::snapshot(&stub.folder)))?;
            out(json!({"snapshot": t.name, "warning": t.warning}));
        }
        "snapshots" => { let dir = owned(st, arg(&rest, 0)?)?.folder; out(json!(folder::snapshots(&dir))); }
        "restore" => {
            let stub = owned(st, arg(&rest, 0)?)?;
            let r = wb_core::container::life(&stub, || {
                if wb_core::container::running(&stub) || stub.open() || (stub.vault && wb_core::vault::mounted(&stub)) { return Err(format!("project {} is open: close it, then restore", stub.id)); }
                folder::restore(&stub.folder, arg(&rest, 1)?)
            })?;
            out(json!({"restored": arg(&rest, 1)?, "before": r.before, "warning": r.warning}));
        }
        "forget" => {
            // Off the host's list; the folder, its data and its snapshots stay where they are.
            let stub = stub_of(st, arg(&rest, 0)?)?;
            if wb_core::container::running(&stub) || stub.open() || (stub.vault && wb_core::vault::mounted(&stub)) { return Err(format!("project {} is open: close it, then forget it", stub.id)); }
            wb_core::roots::remove_stub(&st.data, &stub.id);
            out(json!({"forgotten": stub.id, "folder": stub.folder}));
        }
        "move" => { let to = arg(&rest, 1)?; out(st.move_project(arg(&rest, 0)?, if to == "-" { "" } else { to })?); }
        "for-cwd" => {
            let dir = rest.first().cloned().unwrap_or_else(|| std::env::current_dir().map(|d| d.to_string_lossy().to_string()).unwrap_or_default());
            out(json!(st.project_for_cwd(&absolute(&dir)).map(|p| s(&p, "id").to_string()).unwrap_or_default()));
        }
        "show" => out(json!(st.document(arg(&rest, 0)?, arg(&rest, 1)?))),
        "outline" => {
            let t: Vec<String> = headings(&st.document(arg(&rest, 0)?, arg(&rest, 1)?)).iter().map(|h| format!("{}: {}{}", h.0 + 1, "  ".repeat(h.1 - 1), h.2)).collect();
            out(json!(t.join("\n")));
        }
        "checkout" => out(json!(st.checkout(arg(&rest, 0)?, arg(&rest, 1)?, &session)?.to_string_lossy())),
        "put" => {
            let what = opt(&mut rest, "--what").unwrap_or_default();
            let source = opt(&mut rest, "--source").map(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string())).transpose()?.unwrap_or_else(|| json!({"session": session}));
            let note = opt(&mut rest, "--note").unwrap_or_default();
            out(st.put(arg(&rest, 0)?, arg(&rest, 1)?, &session, &what, Some(source), &note)?);
        }
        "map" => out(st.project_map(arg(&rest, 0)?)),
        "map-put" => { let d = input(&rest)?; out(st.map_put(arg(&rest, 0)?, &d, &session)?); }
        "change-items" => out(st.item_change_detail(arg(&rest, 0)?, arg(&rest, 1)?).unwrap_or_else(|| json!("error: no such change"))),
        "change" => out(st.change_detail(arg(&rest, 0)?, arg(&rest, 2)?, arg(&rest, 1)?).map(Value::Array).unwrap_or_else(|| json!("error: no copy of that change (older than the content limit)"))),
        "take" => {
            let r = st.queue_take(arg(&rest, 0)?, arg(&rest, 1)?, &session)?;
            if !session.is_empty() { session_line(st, &session, r, arg(&rest, 0)?, arg(&rest, 1)?); }
            out(json!(r));
        }
        "done" => { let note = opt(&mut rest, "--note").unwrap_or_default(); st.queue_done(arg(&rest, 0)?, arg(&rest, 1)?, &session, &note)?; out(json!("done")); }
        "queue" => out(json!(st.queue(arg(&rest, 0)?, None))),
        "archive" => { st.archive_doc(arg(&rest, 0)?, arg(&rest, 1)?)?; out(json!("archived")); }
        "recent" => { let d = rest.get(1).and_then(|x| x.parse().ok()).unwrap_or(30); out(json!(st.recent(arg(&rest, 0)?, Some(d)))); }
        _ => { println!("{HELP}"); return Ok(2); }
    }
    Ok(0)
}

/// The take goes in the calling session's own record.
fn session_line(st: &Store, session: &str, result: &str, pid: &str, work: &str) {
    use std::io::Write;
    let why = match result { "taken" => String::new(), "already" => " · why: already done or taken by another session".into(), _ => " · why: no work file".to_string() };
    let dir = st.data.log_sessions();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{session}.log"))) {
        let _ = writeln!(f, "{} [decide] [project] work taken · project={pid} work={work} result={result}{why}", wb_core::log::clock());
    }
}

fn main() {
    paths::private_files();
    let Some(root) = paths::resolve(None, |k| std::env::var(k).ok()) else { eprintln!("error: no data directory"); std::process::exit(1) };
    let st = Store::new(Data::new(root));
    match run(std::env::args().skip(1).collect(), &st) {
        Ok(code) => std::process::exit(code),
        Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
    }
}
