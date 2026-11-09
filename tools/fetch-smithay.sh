#!/bin/sh
# Puts the Smithay source in third_party/smithay with our input method change on it.
#   tools/fetch-smithay.sh           the commit recorded in third_party/smithay.rev
#   tools/fetch-smithay.sh --latest  the newest upstream commit, and records it
# After --latest run the tests; a patch that no longer applies stops here and says so.
set -e
cd "$(dirname "$0")/.."
URL=https://github.com/Smithay/smithay.git
DIR=third_party/smithay
[ -d "$DIR/.git" ] || git clone --quiet "$URL" "$DIR"
git -C "$DIR" fetch --quiet origin
if [ "$1" = "--latest" ]; then
    REV=$(git -C "$DIR" rev-parse --short origin/HEAD)
else
    REV=$(cat third_party/smithay.rev)
fi
git -C "$DIR" reset --quiet --hard "$REV"
git -C "$DIR" clean --quiet -fd
for P in patches/smithay-*.patch; do
    if ! git -C "$DIR" apply "../../$P"; then
        echo "fetch-smithay: $P does not apply to $REV. Fix the patch before moving on." >&2
        exit 1
    fi
done
echo "$REV" > third_party/smithay.rev
echo "smithay $REV"
