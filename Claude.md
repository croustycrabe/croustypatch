## 🚀 Project Overview

**CroustyPatch** is an enterprise-grade and home-lab ready patch management and automated remediation platform developed under the **Croustylab** ecosystem by **Croustycrabe**[cite: 1].


## 🛠️ Tech Stack

- **Backend**: 100% Rust
  - **API & WebSockets**: `actix-web` / `actix`[cite: 4]
  - **ORM & Database**: `diesel` with internal `PostgreSQL`[cite: 4]
  - **Security & Secrets**: Native Rust cryptography (`aes-gcm`), secure memory purging (`minicon`), Argon2id for password hashing[cite: 3, 4].
  - **Protocols**: TLS 1.3, JWT, Custom RSA agent keys, HMAC-SHA256 webhooks, OIDC/SSO, and Model Context Protocol (MCP)[cite: 3, 4].
- **Frontend**: React / Next.js (Modular UI with RBAC, customizable dashboards, and App Portal)[cite: 2, 3, 4, 7].
- **Agent**: Lightweight, root-privileged multi-distribution agent (Linux/FreeBSD) with modular plug-and-play features (Docker, Proxmox Helper Scripts, OpenSCAP, Eco Mode, Pull WebSocket tunnel)[cite: 7, 8].


## 📂 Project Structure (Expected)
```text
/
├── server/               # Rust Actix backend & Diesel ORM
├── agent/                # Rust multi-distribution agent & plugins
├── frontend/             # React / Next.js management interface
├── docs/                 # Technical specifications (.md files)
└── docker/               # Containerization & deployment assets
```

## ⚙️ Development Commands
Backend (Rust)

    Build: cargo build --workspace

    Run Server: cargo run --bin croustypatch-server

    Run Migrations: diesel migration run

    Run Tests: cargo test

##  Frontend (Next.js / React)

    Install Dependencies: npm install (or yarn install / pnpm install)

    Run Dev Server: npm run dev

    Build for Production: npm run build

##  🔒 Core Architectural Rules & Guardrails

    Privilege Isolation: The agent runs as root for system tasks, but command execution must be strictly sandboxed. Standard users cannot trigger updates or bypass security.

    Zero Plaintext Secrets: Sensitive data (API tokens, SMTP secrets) must use column-level encryption (AES-256-GCM) and be wiped from RAM securely.

    Fail-Safe & Double-Failure Protocol: Automated rollbacks must trigger on health-probe failures. If rollbacks fail, immediate network isolation (quarantine) and alerting must occur.

    Modularity: Core components are minimal. Features like Docker monitoring, OpenSCAP compliance, and the App Portal must be modular (activable/deactivable dynamically from the server).

## specification details

All specification about the project can be found undert the /sepcs directory. all document are in french. sepcs files are:

- sepcs/module_notification.md : all information relative to the user notification send
- sepcs/stack_technique: all information relative to which technologie use to create the app
- sepcs/communication: all infroamtions relative to the communication protocol and security between UI, server and agent
- specs/module_reboot.md : all details about the reboot managment option
- sepcs/patch_core: Core function of the software
- specs/portail_app: all about the software installation module
- specs/docker-pve.md: all about the docker discovery and mannagment + proxmox integration
- specs/openscap.md: all about the conformity managment module
- specs/ui: all about the user interface specification
- specs/eco.md: all about the eco mode features for laptop usages
- specs/auth.md : all about the autheications features

specs can be updated with my validation

## Testing

Add all unit and integration test in a /test folder for every crate. Follow the rust convention regarding tests.

## documentation

in the /docs update every features you add form a technical aspect
maintain a changlog.md to trace every changes
