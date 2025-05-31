# Jupiter Exchange API **Rust** Code Examples

This repository showcases fully‑working **Rust** scripts that exercise every Jupiter public API:

* **Swap API** – quote, build & send transactions (standard or via *swap‑instructions*)
* **Ultra API** – place & execute atomic orders with optional referral fees
* **Trigger API** – create & execute on‑chain limit orders
* **Recurring API** – create & execute dollar‑cost‑average (DCA) schedules

Our goal is to provide clear, ready‑to‑run code that developers can copy, extend, and integrate into their own Solana apps.

---

## Getting Started

```bash
#clone this repository
$ git clone https://github.com/Jupiter-DevRel/rust-examples.git
$ cd rust-examples
#Copy the file from .env.example to .env
$ cp .env-example .env
#Build everything once 
$ cargo build --workspace
```

### Running an example

Each flow lives in its own binary crate under `examples/`. Invoke them from the workspace root:

```bash
# Standard quote → swap flow
cargo run -p swap

# Swap‑instructions flow (quote → swap‑instructions → send)
cargo run -p swap_instruction

# Ultra API order & execute
cargo run -p ultra

# Trigger API create & execute
cargo run -p trigger

# Recurring API create & execute
cargo run -p recurring
```

> **Note**
> Trigger and Recurring endpoints enforce minimum order sizes (\~5 USDC and 50 USDC respectively). Increase the example amounts or fund your keypair before running those flows.

---



## Documentation & Resources

* **Developer Docs:** [https://dev.jup.ag/docs](https://dev.jup.ag/docs)
* **API Guides:** [https://dev.jup.ag/docs/api](https://dev.jup.ag/docs/api)
* **Discord:** `#developer-support` channel for quick help

---

## Powered by the Jupiter DevRel Working Group

These examples are maintained by the Jupiter DevRel WG to help onboard and inspire the next wave of Solana builders. We can’t wait to see what you build – feel free to open issues or PRs!

Happy coding! 🚀

---


