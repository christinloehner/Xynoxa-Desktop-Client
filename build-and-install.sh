#!/bin/bash

sudo apt -y purge xynoxa-desktop-client

cd src-tauri && cargo clean && cargo check -q && cd .. && npm run build-app && sudo dpkg -i src-tauri/target/release/bundle/deb/*_amd64.deb
