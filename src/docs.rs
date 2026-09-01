//! Documentation embedded in the binary.
//!
//! `simpled` is most often driven by a coding agent working inside somebody else's
//! project, where the `docs/` directory of this repository is not available. Baking the
//! guides into the binary means `simpled docs ...` is always the same distance away as
//! `--help`, and an agent can look up a field without leaving the shell.

use anyhow::{Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Topic {
    pub name: &'static str,
    pub summary: &'static str,
    pub content: &'static str,
}

pub const TOPICS: &[Topic] = &[
    Topic {
        name: "agent",
        summary: "Condensed operating manual written for AI coding agents",
        content: include_str!("../docs/agent.md"),
    },
    Topic {
        name: "tutorial",
        summary: "Step by step from a blank directory to a running deployment",
        content: include_str!("../docs/tutorial.md"),
    },
    Topic {
        name: "reference",
        summary: "Every appspec.yaml and envspec.yaml field, plus all CLI flags",
        content: include_str!("../docs/reference.md"),
    },
    Topic {
        name: "examples",
        summary: "Annotated real-world configurations covering common patterns",
        content: include_str!("../docs/examples.md"),
    },
    Topic {
        name: "cicd",
        summary: "Automating builds and deployments with GitHub Actions",
        content: include_str!("../docs/cicd.md"),
    },
];

const SKILL_TEMPLATE: &str = include_str!("../docs/skill.md");

fn find_topic(name: &str) -> Result<&'static Topic> {
    let wanted = name.to_ascii_lowercase();
    if let Some(t) = TOPICS.iter().find(|t| t.name == wanted) {
        return Ok(t);
    }
    // A prefix is enough as long as it is unambiguous, so `simpled docs ref` works.
    let matches: Vec<_> = TOPICS.iter().filter(|t| t.name.starts_with(&wanted)).collect();
    match matches.as_slice() {
        [t] => Ok(t),
        [] => bail!(
            "Unknown documentation topic '{}'. Available topics: {}",
            name,
            TOPICS.iter().map(|t| t.name).collect::<Vec<_>>().join(", ")
        ),
        many => bail!(
            "Ambiguous documentation topic '{}'; it matches: {}",
            name,
            many.iter().map(|t| t.name).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// One `##`/`###` section of a document, from its heading to the next heading of the
/// same or a higher level.
struct Section {
    level: usize,
    title: String,
    anchor: String,
    start: usize,
    end: usize,
}

/// GitHub's anchor rules: lower-case, drop everything that is not alphanumeric, a space
/// or a hyphen, then turn spaces into hyphens. An em dash therefore collapses away and
/// leaves the two spaces around it as `--`.
fn anchor_of(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// Headings are only headings outside fenced code blocks — the guides are full of YAML
/// samples whose comments (`# localenv.yaml`) would otherwise parse as `<h1>`.
fn sections(content: &str) -> Vec<Section> {
    let lines: Vec<&str> = content.lines().collect();
    let mut fenced = false;
    let mut out: Vec<Section> = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced || !trimmed.starts_with('#') {
            continue;
        }
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        let title = trimmed[level..].trim();
        if title.is_empty() {
            continue;
        }
        for prev in out.iter_mut().rev() {
            if prev.end > i {
                if prev.level >= level {
                    prev.end = i;
                } else {
                    break;
                }
            }
        }
        out.push(Section {
            level,
            title: title.to_string(),
            anchor: anchor_of(title),
            start: i,
            end: lines.len(),
        });
    }
    out
}

fn slice(content: &str, start: usize, end: usize) -> String {
    content
        .lines()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A section printed on its own does not need the horizontal rule that separated it
/// from the next one in the full document.
fn without_trailing_rule(text: &str) -> &str {
    let trimmed = text.trim_end();
    match trimmed.rsplit_once('\n') {
        Some((head, last)) if last.trim().len() >= 3 && last.trim().chars().all(|c| c == '-') => {
            head.trim_end()
        }
        _ => trimmed,
    }
}

pub fn list() {
    println!("simpled documentation, embedded in the binary.\n");
    println!("Topics:");
    let width = TOPICS.iter().map(|t| t.name.len()).max().unwrap_or(0);
    for topic in TOPICS {
        println!("  {:width$}  {}", topic.name, topic.summary, width = width);
    }
    println!();
    println!("  simpled docs <topic>                print a topic");
    println!("  simpled docs <topic> --outline      list its section headings");
    println!("  simpled docs <topic> --section <s>  print one section");
    println!("  simpled docs search <query>         search every topic");
    println!();
    println!("Start with `simpled docs agent` if you are an AI coding agent.");
}

pub fn show(topic_name: &str, section: Option<&str>, outline: bool) -> Result<()> {
    let topic = find_topic(topic_name)?;

    if outline {
        println!("{} — {}\n", topic.name, topic.summary);
        for s in sections(topic.content) {
            let indent = "  ".repeat(s.level.saturating_sub(1));
            println!("{}{}  (#{})", indent, s.title, s.anchor);
        }
        println!("\nPrint one with: simpled docs {} --section <name>", topic.name);
        return Ok(());
    }

    let Some(wanted) = section else {
        println!("{}", topic.content);
        return Ok(());
    };

    let all = sections(topic.content);
    let needle = wanted.to_lowercase();
    let hits: Vec<&Section> = all
        .iter()
        .filter(|s| s.anchor == anchor_of(wanted) || s.title.to_lowercase().contains(&needle))
        .collect();

    if hits.is_empty() {
        bail!(
            "No section of '{}' matches '{}'. List them with: simpled docs {} --outline",
            topic.name,
            wanted,
            topic.name
        );
    }
    for (i, s) in hits.iter().enumerate() {
        if i > 0 {
            println!("\n---\n");
        }
        println!("{}", without_trailing_rule(&slice(topic.content, s.start, s.end)));
    }
    Ok(())
}

pub fn search(query: &str) -> Result<()> {
    if query.trim().is_empty() {
        bail!("Nothing to search for. Usage: simpled docs search <query>");
    }
    let needle = query.to_lowercase();
    let mut total = 0;

    for topic in TOPICS {
        let all = sections(topic.content);
        let lines: Vec<&str> = topic.content.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            if !line.to_lowercase().contains(&needle) {
                continue;
            }
            // Report the innermost section the hit falls inside, so the reference the
            // caller gets back is the narrowest one they can print.
            let owner = all
                .iter()
                .filter(|s| i >= s.start && i < s.end)
                .max_by_key(|s| s.level);
            match owner {
                Some(s) => println!(
                    "{}#{}\n  {}: {}",
                    topic.name,
                    s.anchor,
                    i + 1,
                    line.trim()
                ),
                None => println!("{}\n  {}: {}", topic.name, i + 1, line.trim()),
            }
            total += 1;
        }
    }

    if total == 0 {
        println!("No matches for '{}'.", query);
        println!("Topics searched: {}", TOPICS.iter().map(|t| t.name).collect::<Vec<_>>().join(", "));
    } else {
        println!("\n{} match(es). Print a section with: simpled docs <topic> --section <name>", total);
    }
    Ok(())
}

/// Writes an agent skill file into a project, so an agent working there discovers
/// `simpled docs` without being told about it.
pub fn init_agent(path: Option<&str>, force: bool, stdout: bool) -> Result<()> {
    let body = SKILL_TEMPLATE.replace("{{VERSION}}", env!("CARGO_PKG_VERSION"));

    if stdout {
        print!("{}", body);
        return Ok(());
    }

    let root = PathBuf::from(path.unwrap_or("."));
    if !root.is_dir() {
        bail!("{} is not a directory", root.display());
    }
    let dir = root.join(".claude").join("skills").join("simpled");
    let file = dir.join("SKILL.md");

    if file.exists() && !force {
        bail!(
            "{} already exists. Pass --force to overwrite it, or --stdout to print the skill instead.",
            file.display()
        );
    }
    fs::create_dir_all(&dir)?;
    fs::write(&file, body)?;
    println!("Wrote {}", file.display());
    println!("Agents working in {} will now find simpled's documentation.", display_root(&root));
    Ok(())
}

fn display_root(root: &Path) -> String {
    match root.canonicalize() {
        // Windows canonicalization returns an extended-length path; the `\\?\` prefix is
        // correct but nobody wants to read it back.
        Ok(p) => p
            .display()
            .to_string()
            .trim_start_matches(r"\\?\")
            .to_string(),
        Err(_) => root.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_has_content() {
        for topic in TOPICS {
            assert!(!topic.content.trim().is_empty(), "{} is empty", topic.name);
        }
    }

    #[test]
    fn topics_resolve_by_name_and_prefix() {
        assert_eq!(find_topic("reference").unwrap().name, "reference");
        assert_eq!(find_topic("REF").unwrap().name, "reference");
        assert!(find_topic("nope").is_err());
    }

    #[test]
    fn anchors_follow_github_rules() {
        assert_eq!(anchor_of("Example 10 — Named volumes"), "example-10--named-volumes");
        assert_eq!(anchor_of("appspec.yaml"), "appspecyaml");
        assert_eq!(anchor_of("CLI reference"), "cli-reference");
    }

    #[test]
    fn headings_inside_code_fences_are_not_sections() {
        let doc = "# Top\n\n```yaml\n# localenv.yaml\nkey: value\n```\n\n## Real\n\nbody\n";
        let found = sections(doc);
        let titles: Vec<&str> = found.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Top", "Real"]);
    }

    #[test]
    fn a_section_ends_at_the_next_heading_of_the_same_level() {
        let doc = "## One\na\n\n### Nested\nb\n\n## Two\nc\n";
        let found = sections(doc);
        let one = found.iter().find(|s| s.title == "One").unwrap();
        assert_eq!(slice(doc, one.start, one.end), "## One\na\n\n### Nested\nb\n");
        let nested = found.iter().find(|s| s.title == "Nested").unwrap();
        assert_eq!(slice(doc, nested.start, nested.end), "### Nested\nb\n");
    }

    #[test]
    fn a_trailing_horizontal_rule_is_dropped() {
        assert_eq!(without_trailing_rule("body\n\n---\n"), "body");
        assert_eq!(
            without_trailing_rule("body\n--- not a rule\n"),
            "body\n--- not a rule"
        );
        assert_eq!(without_trailing_rule("body"), "body");
    }

    #[test]
    fn reference_sections_are_addressable() {
        let reference = find_topic("reference").unwrap();
        let found = sections(reference.content);
        for wanted in ["appspecyaml", "cli-reference", "deployments"] {
            assert!(
                found.iter().any(|s| s.anchor == wanted),
                "reference.md has no section anchored #{}",
                wanted
            );
        }
    }

    #[test]
    fn the_skill_template_is_filled_in() {
        assert!(SKILL_TEMPLATE.contains("{{VERSION}}"));
        assert!(SKILL_TEMPLATE.starts_with("---\n"));
        assert!(SKILL_TEMPLATE.contains("name: simpled"));
    }
}
