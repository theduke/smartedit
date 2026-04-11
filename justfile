help:
    just --list

install:
    cargo install --force --path .
    smartedit install-skill --user
