# Kanari Library Documentation

This documentation is automatically generated from the Move source code.

## Overview

The Kanari Library provides core functionality for the Kanari blockchain ecosystem, including:

- **Token Management**: KARI token implementation with minting, burning, and transfer capabilities
- **Block Operations**: Basic blockchain block structure and operations

## Modules

The following modules are available in this library:

{{#each modules}}
- [{{name}}]({{name}}.md) - {{description}}
{{/each}}

## Getting Started

To use the Kanari Library in your Move projects, add it as a dependency in your `Move.toml`:

```toml
[dependencies]
KanariLibrary = { git = "https://github.com/kanari-network/kanari-sdk.git", subdir = "frameworks/kanari-library" }
```

## License

This project is licensed under the Apache 2.0 License - see the LICENSE file for details.