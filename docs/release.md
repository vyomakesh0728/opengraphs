# OpenGraphs release flow

This repo publishes versioned binaries (`ogtui`, `ogd`) from git tags using `.github/workflows/release.yml`.
Install surfaces expose `og` as an alias to `ogtui`.

## 1. Bump versions

Update these files together:

- `crates/ogtui/Cargo.toml`
- `crates/ogd/Cargo.toml`
- `pyproject.toml`
- `python/agent-chat/pyproject.toml`

## 2. Commit + tag

```bash
git add -A
git commit -m "Release v0.1.4"
git tag v0.1.4
git push origin main
git push origin v0.1.4
```

## 3. GitHub Actions publishes assets

On tag push, CI builds and uploads:

- `opengraphs-v0.1.4-x86_64-unknown-linux-gnu.tar.gz`
- `opengraphs-v0.1.4-aarch64-apple-darwin.tar.gz`
- matching `.sha256` files

## 4. User install/update

Users can install/update to latest:

```bash
curl -fsSL https://raw.githubusercontent.com/vyomakesh0728/opengraphs/main/scripts/install.sh | bash
```

Or pin a version:

```bash
curl -fsSL https://raw.githubusercontent.com/vyomakesh0728/opengraphs/main/scripts/install.sh | bash -s -- --version v0.1.4
```

Or run from npm with npx:

```bash
npx -y opengraphs-cli --help
npx -y opengraphs-cli@0.1.4 --help
```

## 5. Publish npm package

Package source is in `npm/opengraphs/`.

```bash
cd npm/opengraphs
npm publish
```
