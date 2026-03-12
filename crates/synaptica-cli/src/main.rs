pub mod proto {
    tonic::include_proto!("synaptica.client.v1");
}

use anyhow::Result;
use clap::Parser;
use proto::synaptica_service_client::SynapticaServiceClient;
use proto::{
    CreateBackupRequest, DeleteBackupRequest, ExportGraphRequest, GqlValue, HealthRequest,
    ImportGraphRequest, ListBackupsRequest, ListGraphsRequest, QueryRequest,
};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "synaptica", about = "Synaptica GQL database CLI client")]
struct Cli {
    /// gRPC server address
    #[arg(long, default_value = "http://localhost:9090")]
    host: String,

    /// Output format
    #[arg(long, default_value = "table")]
    format: OutputFormat,

    /// Graph name to operate on
    #[arg(long, default_value = "default")]
    graph: String,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum OutputFormat {
    Table,
    Json,
    Csv,
}

fn history_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".synaptica_history")
}

fn format_gql_value(val: &GqlValue) -> String {
    match &val.kind {
        None => "NULL".to_string(),
        Some(proto::gql_value::Kind::NullValue(_)) => "NULL".to_string(),
        Some(proto::gql_value::Kind::BoolValue(b)) => b.to_string(),
        Some(proto::gql_value::Kind::IntegerValue(i)) => i.to_string(),
        Some(proto::gql_value::Kind::FloatValue(f)) => f.to_string(),
        Some(proto::gql_value::Kind::StringValue(s)) => format!("\"{}\"", s),
        Some(proto::gql_value::Kind::BytesValue(b)) => format!("<bytes:{}>", b.len()),
        Some(proto::gql_value::Kind::ListValue(list)) => {
            let items: Vec<String> = list.items.iter().map(format_gql_value).collect();
            format!("[{}]", items.join(", "))
        }
        Some(proto::gql_value::Kind::MapValue(map)) => {
            let entries: Vec<String> = map
                .entries
                .iter()
                .map(|(k, v)| format!("{}: {}", k, format_gql_value(v)))
                .collect();
            format!("{{{}}}", entries.join(", "))
        }
        Some(proto::gql_value::Kind::NodeValue(node)) => {
            let labels = node.labels.join(":");
            let props: Vec<String> = node
                .properties
                .iter()
                .map(|(k, v)| format!("{}: {}", k, format_gql_value(v)))
                .collect();
            format!("({}:{} {{{}}})", node.id, labels, props.join(", "))
        }
        Some(proto::gql_value::Kind::EdgeValue(edge)) => {
            let props: Vec<String> = edge
                .properties
                .iter()
                .map(|(k, v)| format!("{}: {}", k, format_gql_value(v)))
                .collect();
            format!(
                "[{}:{} {}->{} {{{}}}]",
                edge.id,
                edge.label,
                edge.source_id,
                edge.target_id,
                props.join(", ")
            )
        }
    }
}

fn print_table(columns: &[String], rows: &[proto::Row]) {
    if columns.is_empty() {
        return;
    }

    // Compute column widths
    let mut widths: Vec<usize> = columns.iter().map(|c| c.len()).collect();
    let formatted_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            row.values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let s = format_gql_value(v);
                    if i < widths.len() && s.len() > widths[i] {
                        widths[i] = s.len();
                    }
                    s
                })
                .collect()
        })
        .collect();

    // Header
    let header: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{:width$}", c, width = widths[i]))
        .collect();
    let separator: Vec<String> = widths.iter().map(|&w| "-".repeat(w)).collect();

    println!(" {} ", header.join(" | "));
    println!("-{}-", separator.join("-+-"));

    // Rows
    for row in &formatted_rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let w = widths.get(i).copied().unwrap_or(0);
                format!("{:width$}", v, width = w)
            })
            .collect();
        println!(" {} ", cells.join(" | "));
    }
}

fn print_json(columns: &[String], rows: &[proto::Row]) {
    let json_rows: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let mut map = serde_json::Map::new();
            for (i, val) in row.values.iter().enumerate() {
                let key = columns.get(i).cloned().unwrap_or_else(|| i.to_string());
                map.insert(key, serde_json::Value::String(format_gql_value(val)));
            }
            serde_json::Value::Object(map)
        })
        .collect();
    if let Ok(s) = serde_json::to_string_pretty(&json_rows) {
        println!("{}", s);
    }
}

fn print_csv(columns: &[String], rows: &[proto::Row]) {
    println!("{}", columns.join(","));
    for row in rows {
        let cells: Vec<String> = row.values.iter().map(|v| format_gql_value(v)).collect();
        println!("{}", cells.join(","));
    }
}

fn print_help() {
    println!("Synaptica CLI commands:");
    println!("  :help                    Show this help message");
    println!("  :status                  Show server health status");
    println!("  :graphs                  List all available graphs");
    println!("  :backup [label]          Create a backup snapshot");
    println!("  :backups                 List available backups");
    println!("  :delete-backup <name>    Delete a backup");
    println!("  :export <file> [graph]   Export a graph to GQL file");
    println!("  :import <file> [graph]   Import GQL statements from file");
    println!("  :quit, :exit             Exit the CLI");
    println!();
    println!("Enter any GQL query to execute it.");
    println!("Use \\ at end of line for multi-line input.");
    println!("Use --graph <NAME> flag to select a graph (default: 'default').");
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    println!("Connecting to {}...", cli.host);
    let mut client = SynapticaServiceClient::connect(cli.host.clone()).await?;
    println!("Connected. Using graph: {}", cli.graph);

    let prompt = format!("synaptica({})> ", cli.graph);

    let mut rl = DefaultEditor::new()?;
    let hist = history_path();
    let _ = rl.load_history(&hist);

    loop {
        let readline = rl.readline(&prompt);
        match readline {
            Ok(line) => {
                let mut input = line.clone();

                // Multi-line: if line ends with \, continue reading
                while input.ends_with('\\') {
                    input.pop(); // remove trailing backslash
                    match rl.readline("       ...> ") {
                        Ok(cont) => {
                            input.push('\n');
                            input.push_str(&cont);
                        }
                        Err(_) => break,
                    }
                }

                let trimmed = input.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(&input);

                match trimmed {
                    ":quit" | ":exit" => {
                        println!("Goodbye!");
                        break;
                    }
                    ":help" => {
                        print_help();
                    }
                    ":status" => {
                        match client.health(HealthRequest {}).await {
                            Ok(resp) => {
                                let h = resp.into_inner();
                                println!(
                                    "Status: {} | Version: {} | Uptime: {}s",
                                    h.status, h.version, h.uptime_seconds
                                );
                            }
                            Err(e) => {
                                eprintln!("Error: {}", e.message());
                            }
                        }
                    }
                    ":graphs" => {
                        match client.list_graphs(ListGraphsRequest {}).await {
                            Ok(resp) => {
                                let graphs = resp.into_inner().graphs;
                                if graphs.is_empty() {
                                    println!("No graphs found.");
                                } else {
                                    println!("{:<20} {}", "NAME", "ID");
                                    println!("{}", "-".repeat(60));
                                    for g in &graphs {
                                        let marker = if g.name == cli.graph { " *" } else { "" };
                                        println!("{:<20} {}{}", g.name, g.id, marker);
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Error: {}", e.message());
                            }
                        }
                    }
                    query if query.starts_with(":backup") && !query.starts_with(":backups") => {
                        let label = query.strip_prefix(":backup").unwrap().trim();
                        let label = if label.is_empty() { "manual" } else { label };
                        match client
                            .create_backup(CreateBackupRequest {
                                label: label.to_string(),
                            })
                            .await
                        {
                            Ok(resp) => {
                                let r = resp.into_inner();
                                println!("Backup created: {} (at {})", r.backup_name, r.created_at);
                            }
                            Err(e) => eprintln!("Error: {}", e.message()),
                        }
                    }
                    ":backups" => {
                        match client.list_backups(ListBackupsRequest {}).await {
                            Ok(resp) => {
                                let backups = resp.into_inner().backups;
                                if backups.is_empty() {
                                    println!("No backups found.");
                                } else {
                                    println!(
                                        "{:<35} {:<15} {:<25} {}",
                                        "NAME", "LABEL", "CREATED", "SIZE"
                                    );
                                    println!("{}", "-".repeat(85));
                                    for b in &backups {
                                        let size = format_bytes(b.size_bytes);
                                        println!(
                                            "{:<35} {:<15} {:<25} {}",
                                            b.name, b.label, b.created_at, size
                                        );
                                    }
                                }
                            }
                            Err(e) => eprintln!("Error: {}", e.message()),
                        }
                    }
                    query if query.starts_with(":delete-backup") => {
                        let name = query.strip_prefix(":delete-backup").unwrap().trim();
                        if name.is_empty() {
                            eprintln!("Usage: :delete-backup <name>");
                        } else {
                            match client
                                .delete_backup(DeleteBackupRequest {
                                    backup_name: name.to_string(),
                                })
                                .await
                            {
                                Ok(resp) => {
                                    let r = resp.into_inner();
                                    println!("{}", r.message);
                                }
                                Err(e) => eprintln!("Error: {}", e.message()),
                            }
                        }
                    }
                    query if query.starts_with(":export") => {
                        let args: Vec<&str> =
                            query.strip_prefix(":export").unwrap().trim().split_whitespace().collect();
                        if args.is_empty() {
                            eprintln!("Usage: :export <file> [graph]");
                        } else {
                            let file = args[0];
                            let graph = if args.len() > 1 {
                                args[1].to_string()
                            } else {
                                cli.graph.clone()
                            };
                            match client
                                .export_graph(ExportGraphRequest {
                                    graph_name: graph.clone(),
                                })
                                .await
                            {
                                Ok(resp) => {
                                    let mut stream = resp.into_inner();
                                    let mut data = String::new();
                                    while let Ok(Some(chunk)) = stream.message().await {
                                        data.push_str(&chunk.data);
                                    }
                                    match std::fs::write(file, &data) {
                                        Ok(_) => {
                                            let lines = data.lines().count();
                                            println!(
                                                "Exported {} statements from graph '{}' to {}",
                                                lines, graph, file
                                            );
                                        }
                                        Err(e) => eprintln!("Error writing file: {}", e),
                                    }
                                }
                                Err(e) => eprintln!("Error: {}", e.message()),
                            }
                        }
                    }
                    query if query.starts_with(":import") => {
                        let args: Vec<&str> =
                            query.strip_prefix(":import").unwrap().trim().split_whitespace().collect();
                        if args.is_empty() {
                            eprintln!("Usage: :import <file> [graph]");
                        } else {
                            let file = args[0];
                            let graph = if args.len() > 1 {
                                args[1].to_string()
                            } else {
                                cli.graph.clone()
                            };
                            match std::fs::read_to_string(file) {
                                Ok(data) => {
                                    match client
                                        .import_graph(ImportGraphRequest {
                                            graph_name: graph.clone(),
                                            gql_data: data,
                                        })
                                        .await
                                    {
                                        Ok(resp) => {
                                            let r = resp.into_inner();
                                            if let Some(err) = r.error {
                                                eprintln!(
                                                    "Import error after {} statements: {}",
                                                    r.statements_executed, err
                                                );
                                            } else {
                                                println!(
                                                    "Imported {} statements into graph '{}'",
                                                    r.statements_executed, graph
                                                );
                                            }
                                        }
                                        Err(e) => eprintln!("Error: {}", e.message()),
                                    }
                                }
                                Err(e) => eprintln!("Error reading file: {}", e),
                            }
                        }
                    }
                    query => {
                        let request = QueryRequest {
                            query: query.to_string(),
                            graph_name: cli.graph.clone(),
                            parameters: Default::default(),
                            transaction_id: None,
                        };

                        match client.execute_query(request).await {
                            Ok(resp) => {
                                let resp = resp.into_inner();
                                if let Some(err) = &resp.error {
                                    eprintln!("Error: {}", err);
                                } else {
                                    match cli.format {
                                        OutputFormat::Table => {
                                            print_table(&resp.columns, &resp.rows)
                                        }
                                        OutputFormat::Json => {
                                            print_json(&resp.columns, &resp.rows)
                                        }
                                        OutputFormat::Csv => {
                                            print_csv(&resp.columns, &resp.rows)
                                        }
                                    }
                                    if let Some(stats) = &resp.stats {
                                        println!(
                                            "\n{} rows returned in {}ms",
                                            stats.rows_returned, stats.execution_time_ms
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Error: {}", e.message());
                            }
                        }
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("Goodbye!");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("Goodbye!");
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    let _ = rl.save_history(&hist);
    Ok(())
}
