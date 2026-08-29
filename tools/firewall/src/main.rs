//! # bwfw — Black Wall Core Firewall CLI
//!
//! Manages firewall rules stored in `/etc/bwfw/rules.toml`.
//! Rules are applied to the kernel network stack via the `net` crate's
//! firewall module at boot time (called by `bwinit`).
//!
//! ## Commands
//! - `bwfw list`               — list active rules
//! - `bwfw add <rule>`         — add a rule
//! - `bwfw del <id>`           — delete a rule by index
//! - `bwfw enable`             — enable the firewall
//! - `bwfw disable`            — disable the firewall
//! - `bwfw apply <file>`       — apply rules from a TOML file
//! - `bwfw reset`              — reset to default (deny all / allow SSH)
//! - `bwfw status`             — show firewall status

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::Path;

const RULES_PATH: &str = "/etc/bwfw/rules.toml";

// ─── Rule model ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Deny,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::Allow => write!(f, "allow"),
            Action::Deny => write!(f, "deny"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RuleSet {
    pub enabled: bool,
    pub default_policy: Action,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Default for RuleSet {
    fn default() -> Self {
        // Default: deny all inbound except SSH.
        RuleSet {
            enabled: true,
            default_policy: Action::Deny,
            rules: vec![
                Rule {
                    action: Action::Allow,
                    protocol: Some("tcp".to_string()),
                    src: None,
                    dst: None,
                    dst_port: Some(22),
                    comment: Some("Allow SSH".to_string()),
                },
                Rule {
                    action: Action::Allow,
                    protocol: Some("icmp".to_string()),
                    src: None,
                    dst: None,
                    dst_port: None,
                    comment: Some("Allow ICMP (ping)".to_string()),
                },
            ],
        }
    }
}

// ─── Persistence ──────────────────────────────────────────────────────────────

fn load_rules() -> Result<RuleSet> {
    if !Path::new(RULES_PATH).exists() {
        return Ok(RuleSet::default());
    }
    let raw = std::fs::read_to_string(RULES_PATH).context("Failed to read rules file")?;
    toml::from_str(&raw).context("Failed to parse rules file")
}

fn save_rules(rules: &RuleSet) -> Result<()> {
    std::fs::create_dir_all("/etc/bwfw").context("Failed to create /etc/bwfw")?;
    let toml = toml::to_string_pretty(rules).context("Failed to serialize rules")?;
    std::fs::write(RULES_PATH, toml).context("Failed to write rules file")?;
    Ok(())
}

// ─── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "bwfw", about = "Black Wall Core Firewall")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all firewall rules
    List,
    /// Show firewall status
    Status,
    /// Enable the firewall
    Enable,
    /// Disable the firewall
    Disable,
    /// Add a rule
    Add {
        /// Action: allow|deny
        action: String,
        /// Protocol: tcp|udp|icmp|any
        #[arg(long, default_value = "any")]
        proto: String,
        /// Source CIDR (e.g. 192.168.1.0/24)
        #[arg(long)]
        src: Option<String>,
        /// Destination port
        #[arg(long, short = 'p')]
        port: Option<u16>,
        /// Comment
        #[arg(long, short = 'c')]
        comment: Option<String>,
    },
    /// Delete a rule by index (0-based)
    Del { index: usize },
    /// Reset to default rules (deny all / allow SSH)
    Reset,
    /// Apply rules from a TOML file
    Apply { file: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::List => cmd_list(),
        Commands::Status => cmd_status(),
        Commands::Enable => cmd_set_enabled(true),
        Commands::Disable => cmd_set_enabled(false),
        Commands::Add { action, proto, src, port, comment } => {
            cmd_add(action, proto, src, port, comment)
        }
        Commands::Del { index } => cmd_del(index),
        Commands::Reset => cmd_reset(),
        Commands::Apply { file } => cmd_apply(file),
    }
}

fn cmd_list() -> Result<()> {
    let rs = load_rules()?;
    let status = if rs.enabled { "ENABLED".bright_green() } else { "DISABLED".bright_red() };
    println!("Firewall: {} | Default policy: {}", status, rs.default_policy.to_string().bold());
    println!();
    println!(
        "{:>4}  {:<6} {:<8} {:<20} {:<8} {}",
        "ID".bold(),
        "ACTION".bold(),
        "PROTO".bold(),
        "SRC".bold(),
        "PORT".bold(),
        "COMMENT".bold()
    );
    println!("{}", "─".repeat(64).dimmed());
    for (i, rule) in rs.rules.iter().enumerate() {
        let action_str = match rule.action {
            Action::Allow => "allow".bright_green().to_string(),
            Action::Deny => "deny".bright_red().to_string(),
        };
        println!(
            "{:>4}  {:<14} {:<8} {:<20} {:<8} {}",
            i,
            action_str,
            rule.protocol.as_deref().unwrap_or("any"),
            rule.src.as_deref().unwrap_or("any"),
            rule.dst_port.map(|p| p.to_string()).unwrap_or_else(|| "any".to_string()),
            rule.comment.as_deref().unwrap_or("")
        );
    }
    Ok(())
}

fn cmd_status() -> Result<()> {
    let rs = load_rules()?;
    if rs.enabled {
        println!("{} Firewall is ENABLED", "●".bright_green());
    } else {
        println!("{} Firewall is DISABLED", "●".bright_red());
    }
    println!("  Default policy : {}", rs.default_policy);
    println!("  Active rules   : {}", rs.rules.len());
    Ok(())
}

fn cmd_set_enabled(enabled: bool) -> Result<()> {
    let mut rs = load_rules()?;
    rs.enabled = enabled;
    save_rules(&rs)?;
    if enabled {
        println!("{} Firewall enabled.", "✓".bright_green());
    } else {
        println!("{} Firewall disabled.", "!".bright_yellow());
    }
    Ok(())
}

fn cmd_add(
    action: String,
    proto: String,
    src: Option<String>,
    port: Option<u16>,
    comment: Option<String>,
) -> Result<()> {
    let action = match action.to_lowercase().as_str() {
        "allow" => Action::Allow,
        "deny" => Action::Deny,
        _ => anyhow::bail!("Action must be 'allow' or 'deny'"),
    };
    let protocol = if proto == "any" { None } else { Some(proto) };

    let rule = Rule { action, protocol, src, dst: None, dst_port: port, comment };
    let mut rs = load_rules()?;
    rs.rules.push(rule);
    save_rules(&rs)?;
    println!("{} Rule added (ID {}).", "✓".bright_green(), rs.rules.len() - 1);
    Ok(())
}

fn cmd_del(index: usize) -> Result<()> {
    let mut rs = load_rules()?;
    if index >= rs.rules.len() {
        anyhow::bail!("No rule at index {}", index);
    }
    rs.rules.remove(index);
    save_rules(&rs)?;
    println!("{} Rule {} deleted.", "✓".bright_green(), index);
    Ok(())
}

fn cmd_reset() -> Result<()> {
    let rs = RuleSet::default();
    save_rules(&rs)?;
    println!("{} Firewall reset to defaults (deny all / allow SSH).", "✓".bright_green());
    Ok(())
}

fn cmd_apply(file: String) -> Result<()> {
    let raw = std::fs::read_to_string(&file).with_context(|| format!("Failed to read {}", file))?;
    let rs: RuleSet = toml::from_str(&raw).context("Invalid rules file")?;
    save_rules(&rs)?;
    println!("{} Applied {} rules from {}.", "✓".bright_green(), rs.rules.len(), file);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ruleset_has_ssh_and_icmp() {
        let rs = RuleSet::default();
        assert!(rs.enabled);
        assert_eq!(rs.default_policy, Action::Deny);
        assert_eq!(rs.rules.len(), 2);
        assert_eq!(rs.rules[0].dst_port, Some(22));
        assert_eq!(rs.rules[0].protocol.as_deref(), Some("tcp"));
        assert_eq!(rs.rules[1].protocol.as_deref(), Some("icmp"));
    }

    #[test]
    fn ruleset_roundtrip_toml() {
        let rs = RuleSet::default();
        let toml_str = toml::to_string_pretty(&rs).unwrap();
        let loaded: RuleSet = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.enabled, rs.enabled);
        assert_eq!(loaded.default_policy, rs.default_policy);
        assert_eq!(loaded.rules.len(), rs.rules.len());
    }

    #[test]
    fn add_rule_to_ruleset() {
        let mut rs = RuleSet::default();
        rs.rules.push(Rule {
            action: Action::Deny,
            protocol: Some("udp".to_string()),
            src: None,
            dst: None,
            dst_port: Some(53),
            comment: Some("Block DNS".to_string()),
        });
        assert_eq!(rs.rules.len(), 3);
        assert_eq!(rs.rules[2].dst_port, Some(53));
    }

    #[test]
    fn delete_rule() {
        let mut rs = RuleSet::default();
        rs.rules.remove(0);
        assert_eq!(rs.rules.len(), 1);
        assert_eq!(rs.rules[0].protocol.as_deref(), Some("icmp"));
    }

    #[test]
    fn action_display() {
        assert_eq!(format!("{}", Action::Allow), "allow");
        assert_eq!(format!("{}", Action::Deny), "deny");
    }

    #[test]
    fn empty_ruleset_roundtrip() {
        let rs = RuleSet { enabled: false, default_policy: Action::Allow, rules: vec![] };
        let toml_str = toml::to_string_pretty(&rs).unwrap();
        let loaded: RuleSet = toml::from_str(&toml_str).unwrap();
        assert!(!loaded.enabled);
        assert_eq!(loaded.default_policy, Action::Allow);
        assert!(loaded.rules.is_empty());
    }

    #[test]
    fn rule_serialization_roundtrip_v2() {
        let rule = Rule {
            action: Action::Allow,
            protocol: Some("tcp".to_string()),
            src: Some("10.0.0.0/8".to_string()),
            dst: None,
            dst_port: Some(443),
            comment: None,
        };
        let toml_str = toml::to_string(&rule).unwrap();
        let loaded: Rule = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.action, Action::Allow);
        assert_eq!(loaded.protocol.as_deref(), Some("tcp"));
        assert_eq!(loaded.src.as_deref(), Some("10.0.0.0/8"));
        assert_eq!(loaded.dst, None);
        assert_eq!(loaded.dst_port, Some(443));
        assert_eq!(loaded.comment, None);
    }

    #[test]
    fn ruleset_empty_serialization() {
        let rs = RuleSet { enabled: true, default_policy: Action::Deny, rules: vec![] };
        let toml_str = toml::to_string_pretty(&rs).unwrap();
        let loaded: RuleSet = toml::from_str(&toml_str).unwrap();
        assert!(loaded.enabled);
        assert_eq!(loaded.default_policy, Action::Deny);
        assert!(loaded.rules.is_empty());
        assert_eq!(loaded.rules.len(), 0);
    }

    #[test]
    fn rule_missing_optional_fields() {
        let rule = Rule {
            action: Action::Deny,
            protocol: None,
            src: None,
            dst: None,
            dst_port: Some(8080),
            comment: None,
        };
        let toml_str = toml::to_string(&rule).unwrap();
        assert!(!toml_str.contains("protocol"));
        assert!(!toml_str.contains("src"));
        assert!(!toml_str.contains("dst\n"));
        assert!(!toml_str.contains("comment"));
        let loaded: Rule = toml::from_str(&toml_str).unwrap();
        assert!(loaded.protocol.is_none());
        assert!(loaded.src.is_none());
        assert!(loaded.dst.is_none());
        assert!(loaded.comment.is_none());
        assert_eq!(loaded.dst_port, Some(8080));
        assert_eq!(loaded.action, Action::Deny);
    }

    #[test]
    fn default_policy_allow() {
        let rs = RuleSet { enabled: true, default_policy: Action::Allow, rules: vec![] };
        let toml_str = toml::to_string_pretty(&rs).unwrap();
        assert!(toml_str.contains("default_policy = \"allow\""));
        let loaded: RuleSet = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.default_policy, Action::Allow);
        assert_eq!(loaded.default_policy.to_string(), "allow");
    }

    #[test]
    fn rule_multiple_protocols() {
        for proto in ["tcp", "udp", "icmp"] {
            let rule = Rule {
                action: Action::Allow,
                protocol: Some(proto.to_string()),
                src: None,
                dst: None,
                dst_port: None,
                comment: None,
            };
            let toml_str = toml::to_string_pretty(&rule).unwrap();
            assert!(toml_str.contains(&format!("protocol = \"{}\"", proto)));
            let loaded: Rule = toml::from_str(&toml_str).unwrap();
            assert_eq!(loaded.protocol.as_deref(), Some(proto));
            assert_eq!(loaded.action, Action::Allow);
            assert!(loaded.dst_port.is_none());
        }
    }

    #[test]
    fn action_variants() {
        for (variant, expected) in [(Action::Allow, "allow"), (Action::Deny, "deny")] {
            assert_eq!(variant.to_string(), expected);
            let rule = Rule {
                action: variant.clone(),
                protocol: None,
                src: None,
                dst: None,
                dst_port: None,
                comment: None,
            };
            let toml_str = toml::to_string(&rule).unwrap();
            assert!(toml_str.contains(&format!("action = \"{}\"", expected)));
            let loaded: Rule = toml::from_str(&toml_str).unwrap();
            assert_eq!(loaded.action, variant);
        }
    }
}
