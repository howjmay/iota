#!/bin/sh
# Copyright (c) Mysten Labs, Inc.
# Modifications Copyright (c) 2024 IOTA Stiftung
# SPDX-License-Identifier: Apache-2.0

# fast fail.
set -e

DIR="$( cd "$( dirname "$0" )" && pwd )"
REPO_ROOT="$(git rev-parse --show-toplevel)"
OCI_OUTPUT="$REPO_ROOT/build/oci"
DOCKERFILE="$DIR/Dockerfile"
GIT_REVISION="$(git describe --always --abbrev=12 --dirty --exclude '*')"
BUILD_DATE="$(date -u +'%Y-%m-%d')"
PROFILE="release"
PLATFORM="linux/amd64"

echo
echo "Building minimal deterministic repro"
echo "Dockerfile: \t$DOCKERFILE"
echo "docker context: $REPO_ROOT"
echo "build date: \t$BUILD_DATE"
echo "git revision: \t$GIT_REVISION"
echo "output directory: \t$OCI_OUTPUT"
echo

export DOCKER_BUILDKIT=1
export SOURCE_DATE_EPOCH=1

# TODO: Remove "--ssh default" when iota-sim is public https://github.com/iotaledger/iota/issues/2149
docker build --ssh default -f "$DOCKERFILE" "$REPO_ROOT" \
	--build-arg PROFILE="$PROFILE" \
	--platform "$PLATFORM" \
	--output type=oci,rewrite-timestamp=true,force-compression=true,tar=false,dest=$OCI_OUTPUT/canary,name=canary \
	"$@"
