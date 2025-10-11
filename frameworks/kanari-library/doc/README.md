
<a name="@Kanari_Library_Documentation_0"></a>

# Kanari Library Documentation


This documentation is automatically generated from the Move source code.


<a name="@Overview_1"></a>

## Overview


The Kanari Library provides core functionality for the Kanari blockchain ecosystem, including:

- **Token Management**: KARI token implementation with minting, burning, and transfer capabilities
- **Block Operations**: Basic blockchain block structure and operations


<a name="@Modules_2"></a>

## Modules


The following modules are available in this library:

{{#each modules}}
- [{{name}}]({{name}}.md) - {{description}}
{{/each}}


<a name="@Getting_Started_3"></a>

## Getting Started


To use the Kanari Library in your Move projects, add it as a dependency in your <code>Move.toml</code>:

```toml
[dependencies]
KanariLibrary = { git = "https://github.com/kanari-network/kanari-sdk.git", subdir = "frameworks/kanari-library" }
```


<a name="@License_4"></a>

## License


This project is licensed under the Apache 2.0 License - see the LICENSE file for details.
