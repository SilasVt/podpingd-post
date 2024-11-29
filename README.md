# podpingd

`podpingd` is a daemon to sync podpings to a variety of storage mediums.

## Running

To start, you need cargo. The most basic command is:

```shell
# This is a default value set in conf/00-default.toml
mkdir ./data
cargo run --release
```

## With config file

Alternatively, copy a config file from the conf directory and modify it to your needs.

```shell
cp conf/00-default.toml conf/10-user.toml
$EDITOR conf/10-user.toml
PODPINGD_CONFIG_FILE=conf/10-user.toml cargo run --release
```

## Using environment variables

Values from config files can be set directly.

Use `__` (two underscores) to separate values from the config hierarchy.

```shell
mkdir ./podping-data
# Note how disk_directory contains one underscore because it's the variable being set
PODPINGD__WRITER__DISK_DIRECTORY="./podping-data" cargo run --release
```

## Docker/containers

Coming soon.
