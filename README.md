# eww_prayer_times

A simple prayer time notifier and widget for `eww` (ElKowar's Wacky Widgets), written in Rust. It calculates prayer times for any location (working completely offline), runs as an efficient background daemon, and sends desktop notifications when prayer time arrives.

![Screenshot](assets/1.png)
![Screenshot](assets/2.png)

## Usage

1.  Build the binary:
    ```sh
    cargo build --release
    ```

2.  The example `eww` widget configuration is available in the `eww/` directory. You may need to adjust the binary path inside `eww.yuck`.

## Data Source

The city database used for the `--city` flag is sourced from [lutangar/cities.json](https://github.com/lutangar/cities.json).

## License

MIT ([LICENSE](LICENSE))