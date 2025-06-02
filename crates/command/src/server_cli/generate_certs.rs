// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use colored::Colorize;
use common::get_kari_dir;
use std::process::Command;

pub fn generate_ssl_certificates() -> Result<(), String> {
    println!(
        "{}",
        "Generating SSL certificates for secure connections..."
            .bright_green()
            .bold()
    );

    // Get the Kari certificates directory
    let kari_dir = get_kari_dir();
    let certs_dir = kari_dir.join("certs");

    // Create the directory if it doesn't exist
    if !certs_dir.exists() {
        std::fs::create_dir_all(&certs_dir)
            .map_err(|e| format!("Failed to create certificates directory: {}", e))?;
    }

    // Define certificate paths
    let cert_path = certs_dir.join("node.crt");
    let key_path = certs_dir.join("node.key");

    // Check if certificates already exist
    if cert_path.exists() && key_path.exists() {
        println!("{}", "SSL certificates already exist:".yellow());
        println!(
            "  - Certificate: {}",
            cert_path.display().to_string().bright_white()
        );
        println!("  - Key: {}", key_path.display().to_string().bright_white());

        println!("\nDo you want to generate new certificates? (y/N): ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Certificate generation cancelled.".yellow());
            return Ok(());
        }
    }

    // Get hostname for certificate
    let hostname = match Command::new("hostname")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    {
        Ok(name) => name,
        Err(_) => "kanari-node".to_string(),
    };

    println!(
        "Generating certificate for node: {}",
        hostname.bright_white()
    );

    // Create command to generate certificates with OpenSSL
    #[cfg(not(target_os = "windows"))]
    let status = Command::new("openssl")
        .args(&[
            "req",
            "-x509",
            "-newkey",
            "rsa:4096",
            "-keyout",
            key_path.to_str().unwrap(),
            "-out",
            cert_path.to_str().unwrap(),
            "-days",
            "365",
            "-nodes",
            "-subj",
            &format!("/CN={}", hostname),
            "-addext",
            &format!("subjectAltName=DNS:{}", hostname),
        ])
        .status();

    #[cfg(target_os = "windows")]
    let status = Command::new("openssl")
        .args(&[
            "req",
            "-x509",
            "-newkey",
            "rsa:4096",
            "-keyout",
            key_path.to_str().unwrap(),
            "-out",
            cert_path.to_str().unwrap(),
            "-days",
            "365",
            "-nodes",
            "-subj",
            &format!("//CN={}", hostname),
            "-addext",
            &format!("subjectAltName=DNS:{}", hostname),
        ])
        .status();

    match status {
        Ok(exit_status) => {
            if exit_status.success() {
                println!(
                    "\n{}",
                    "Successfully generated SSL certificates:"
                        .bright_green()
                        .bold()
                );
                println!(
                    "  - Certificate: {}",
                    cert_path.display().to_string().bright_white()
                );
                println!("  - Key: {}", key_path.display().to_string().bright_white());
                println!("\nCertificates will be valid for 365 days.");
                Ok(())
            } else {
                Err(format!(
                    "Failed to generate certificates (exit code: {})",
                    exit_status
                ))
            }
        }
        Err(e) => Err(format!(
            "Failed to execute openssl command: {}\nMake sure OpenSSL is installed on your system.",
            e
        )),
    }
}
