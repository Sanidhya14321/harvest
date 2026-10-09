#!/usr/bin/env bash
set -euo pipefail

tag=${1:-}
mode=${2:-}
if [[ "$#" -gt 2 ]] || { [[ -n "${mode}" ]] && [[ "${mode}" != --dry-run ]]; }; then
    printf 'usage: submit_winget.sh [vVERSION] [--dry-run]\n' >&2
    exit 1
fi

source_repo=${GITHUB_REPOSITORY:-Sanidhya14321/harvest}
fork=pulkitxm/winget-pkgs
upstream=microsoft/winget-pkgs
if [[ -z "${tag}" ]]; then
    tag=$(gh api "repos/${source_repo}/releases/latest" --jq .tag_name)
fi
if [[ ! ${tag} =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    printf 'not a stable release tag: %s\n' "${tag}" >&2
    exit 1
fi
version=${tag#v}
branch="new-pulkitxm-omp-${version}"
manifest_path="manifests/p/Pulkitxm/Omp/${version}"

work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/omp-winget.XXXXXX")
trap 'rm -rf "${work}"' EXIT
gh api "repos/${source_repo}/releases/tags/${tag}" >"${work}/release.json"
jq -e '.draft == false and .prerelease == false' "${work}/release.json" >/dev/null
gh release download "${tag}" --repo "${source_repo}" --dir "${work}" \
    --pattern SHA256SUMS --pattern omp-winget-manifests.zip --pattern omp-windows-x86_64.zip

(
    cd "${work}"
    awk '$2 == "omp-winget-manifests.zip" || $2 == "omp-windows-x86_64.zip"' SHA256SUMS >checksums
    count=$(wc -l <checksums | tr -d ' ')
    test "${count}" = 2
    shasum -a 256 --check checksums
)
archive="${work}/omp-winget-manifests.zip"
expected=$(printf '%s\n' Pulkitxm.Omp.installer.yaml Pulkitxm.Omp.locale.en-US.yaml Pulkitxm.Omp.yaml | sort)
contents=$(unzip -Z1 "${archive}" | sort)
test "${contents}" = "${expected}"
mkdir "${work}/manifests"
for file in Pulkitxm.Omp.installer.yaml Pulkitxm.Omp.locale.en-US.yaml Pulkitxm.Omp.yaml; do
    unzip -p "${archive}" "${file}" >"${work}/manifests/${file}"
    grep -Fx 'PackageIdentifier: Pulkitxm.Omp' "${work}/manifests/${file}" >/dev/null
    grep -Fx "PackageVersion: \"${version}\"" "${work}/manifests/${file}" >/dev/null
done
checksum=$(awk '$2 == "omp-windows-x86_64.zip" {print toupper($1)}' "${work}/SHA256SUMS")
grep -Fx "    InstallerSha256: \"${checksum}\"" "${work}/manifests/Pulkitxm.Omp.installer.yaml" >/dev/null
grep -Fx "    InstallerUrl: https://github.com/${source_repo}/releases/download/${tag}/omp-windows-x86_64.zip" \
    "${work}/manifests/Pulkitxm.Omp.installer.yaml" >/dev/null

git clone --quiet --depth=1 --filter=blob:none --sparse "https://github.com/${upstream}.git" "${work}/repository"
git -C "${work}/repository" sparse-checkout set manifests/p/Pulkitxm/Omp
cd "${work}/repository"
if [[ -d "${manifest_path}" ]]; then
    printf 'WinGet already contains Pulkitxm.Omp %s\n' "${version}"
    exit 0
fi
existing=$(gh api "repos/${upstream}/pulls?head=pulkitxm:${branch}&state=open" --jq '.[0].html_url // empty')
if [[ -n "${existing}" ]]; then
    printf 'WinGet submission already open: %s\n' "${existing}"
    exit 0
fi

git remote set-url origin "https://github.com/${fork}.git"
if git ls-remote --exit-code origin "refs/heads/${branch}" >/dev/null; then
    git fetch --quiet --depth=1 origin "${branch}"
    git switch --quiet -c "${branch}" FETCH_HEAD
else
    sha=$(git rev-parse HEAD)
    arguments=(repository sync-fork --repo "${fork}" --branch master --json)
    if [[ "${mode}" = --dry-run ]]; then arguments+=(--dry-run); fi
    pukbot "${arguments[@]}"
    if [[ "${mode}" = --dry-run ]]; then
        pukbot ref create "refs/heads/${branch}" --repo "${fork}" --sha "${sha}" --dry-run --json
    else
        pukbot ref create "refs/heads/${branch}" --repo "${fork}" --sha "${sha}" --json
    fi
    git switch --quiet -c "${branch}"
fi
mkdir -p "${manifest_path}"
cp "${work}/manifests/"*.yaml "${manifest_path}/"
git add "${manifest_path}"
if ! git diff --cached --quiet; then
    arguments=(commit create --repo "${fork}" --branch "${branch}" --message "chore: submit Omp ${version} to WinGet" --json)
    if [[ "${mode}" = --dry-run ]]; then arguments+=(--dry-run); fi
    pukbot "${arguments[@]}"
fi
arguments=(pr create --repo "${upstream}" --head "pulkitxm:${branch}" --base master
    --title "Update: Pulkitxm.Omp to ${version}"
    --body "Updates Omp to ${version} using its released manifests and checksum-verified Windows archive." --json)
if [[ "${mode}" = --dry-run ]]; then arguments+=(--dry-run); fi
pukbot "${arguments[@]}"
