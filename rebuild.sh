#!/bin/bash

cargo build --release || exit
sudo systemctl restart speaker
