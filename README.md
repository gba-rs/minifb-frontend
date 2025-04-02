# minifb-frontend

![Rust Logo](https://www.rust-lang.org/logos/rust-logo-256x256.png)

A minimalistic frontend for the GBA emulator (`gba-emu`) using the `minifb` crate for rendering.

## Description

The `minifb-frontend` provides a simple and lightweight graphical interface for running the `gba-emu` emulator. It uses the `minifb` crate to create a window and display the emulation output. This frontend is ideal for users who want a straightforward way to run the emulator on their desktop.

## Features

- Minimalistic and lightweight
- Uses the `minifb` crate for rendering
- Compatible with the `gba-emu` emulator
- Easy to set up and run

## Prerequisites

Before using this frontend, ensure you have the following installed:

- Rust (nightly version)
- The `gba-emu` emulator
- A GBA ROM file to test the emulator

## Installation

1. Clone the repository:

    ```bash
    git clone https://github.com/gba-rs/minifb-frontend.git
    ```

2. Navigate into the project directory:

    ```bash
    cd minifb-frontend
    ```

3. Build the project:

    ```bash
    cargo build
    ```

## Usage

To run the emulator with this frontend:

1. Ensure you have the `gba-emu` emulator built and accessible.
2. Run the frontend with the path to your GBA ROM file:

    ```bash
    cargo run --release your_bios your_rom.gba
    ```

    Replace `your_rom.gba` with the path to your GBA ROM file.
    Replase `your_bios` with your bios file.

## Dependencies

- Rust (nightly version)
- `minifb` crate for rendering
- A GBA ROM file to test the emulator

## Contributing

Contributions are welcome! If you have any improvements or bug fixes, feel free to open a pull request.

## License

This project is licensed under either of the following licenses:

- Apache License, Version 2.0, [LICENSE_APACHE](LICENSE_APACHE)
- MIT License, [LICENSE_MIT](LICENSE_MIT)

You may choose one of them.

## Acknowledgements

- Thanks to the contributors of the `gba-emu` project and the Rust community for their support.