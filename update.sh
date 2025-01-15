#!/usr/bin/env bash

if [ $# -lt 1 ]; then
    echo "ERROR: upload.sh missing args"
    exit 1
fi

SOURCE_FILE="$1"
FILE_NAME="$(basename "$SOURCE_FILE")"
EXTENSION="${FILE_NAME##*.}"

NEW_NAME="$(cat /proc/sys/kernel/random/uuid).$EXTENSION"
BUCKET="YOUR_BUCKET"
REGION="YOUR_REGION"

sudo cp "$SOURCE_FILE" "./$NEW_NAME"
sudo chown "$(whoami):" "./$NEW_NAME"
aws s3 cp "./$NEW_NAME" "s3://$BUCKET" > /dev/null
rm "./$NEW_NAME"
echo -n "https://${BUCKET}.s3.${REGION}.amazonaws.com/${NEW_NAME}"
