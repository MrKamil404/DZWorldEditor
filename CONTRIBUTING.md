# Contributing to DZ World Editor

Thank you for your interest in contributing to **DZ World Editor**! Whether you are reporting bugs, suggesting new features, improving documentation, or submitting code, your help is appreciated.

## Reporting Bugs

Before creating a new issue, please check the existing [Issues](https://github.com/MrKamil404/DZWorldEditor/issues) to see if the problem has already been reported.

When reporting a bug, please include:
* **Editor version** (e.g., from the latest release or the exact commit hash if built from source).
* **Operating System** (e.g., Windows 10 / Windows 11).
* **Steps to reproduce** the issue clearly.
* **Expected vs. actual behavior**.
* **Logs, crash reports, or screenshots** (if applicable).

## Suggesting Features

If you have an idea for a new generator, tool, or workflow improvement:
* Open an issue and clearly describe the problem it solves and how it should work inside the editor.
* Keep in mind DayZ engine limitations (such as Terrain Builder compatibility, `CfgSurfaces` per-tile limits, and PBO/AddonBuilder constraints).

## Development Setup

To build and test **DZ World Editor** locally from source:

1. **Prerequisites:**
   * [Rust](https://www.rust-lang.org/tools/install) (latest stable toolchain recommended).
   * Windows 10/11.
   * DayZ Tools installed with a configured `P:\` drive (required for testing object libraries, textures, and PBO building).

2. **Clone and build:**
   ```bash
   git clone https://github.com/MrKamil404/DZWorldEditor.git
   cd DZWorldEditor
   cargo build
