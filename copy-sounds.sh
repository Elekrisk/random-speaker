#!/bin/bash

./normalize.sh
./remove-not-in-normalize.sh

rsync -avh sounds/ speaker:random-speaker/sounds --update --delete
