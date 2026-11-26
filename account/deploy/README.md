# Deploying account.nuzky.app

Three containers on the Oracle Cloud VM (Ubuntu 24.04, aarch64, Docker with the compose plugin): Caddy on ports 80 and 443 with automatic TLS, the account server, which only Caddy reaches, and Litestream, which copies every change of the SQLite database to OCI Object Storage within seconds. The database lives in the `data` volume.

## Steps only the maintainer can do

1. **OCI network.** In the VM's subnet security list (or its network security group), add ingress rules for TCP 80 and TCP 443 from `0.0.0.0/0`, and UDP 443 for HTTP/3 if you want it. The Ubuntu image also has its own iptables rules that drop everything but SSH; open the ports there too and keep them after a reboot:

   ```sh
   sudo iptables -I INPUT 6 -m state --state NEW -p tcp --dport 80 -j ACCEPT
   sudo iptables -I INPUT 6 -m state --state NEW -p tcp --dport 443 -j ACCEPT
   sudo iptables -I INPUT 6 -m state --state NEW -p udp --dport 443 -j ACCEPT
   sudo netfilter-persistent save
   ```

2. **DNS.** In Cloudflare add `A account <the VM's public IPv4>` with the proxy off (DNS only, grey cloud), so Caddy gets its certificate and the server sees the real client address.

3. **Google.** In Google Cloud Console, in a project for Nuzky:
   - OAuth consent screen: app name Nuzky, support email, the logo, `https://nuzky.app` as the home page, its privacy policy and terms, authorized domain `nuzky.app`, scopes `openid`, `email` and `profile` only. Publish it to production; these scopes need no verification.
   - Credentials → Create OAuth client ID → Web application, with the authorized redirect URI `https://account.nuzky.app/api/auth/callback/google` and no JavaScript origins. Put its client ID and secret in `.env`.

4. **Email.** Choose an SMTP provider and verify `nuzky.app` (or a subdomain such as `mail.nuzky.app`) there. Add the SPF and DKIM records it gives you in Cloudflare, plus DMARC, for example `TXT _dmarc "v=DMARC1; p=quarantine; rua=mailto:<you>"`. Put the SMTP address in `SMTP_URL` (`smtps://user:password@host:465`) and the sender in `MAIL_FROM`.

5. **Backups.** In OCI Object Storage create a private bucket, for example `nuzky-account-backup`, in the VM's region. Under your user (better: a user that may only write to that bucket) create a Customer Secret Key; its access key and secret are the S3 credentials. The endpoint is `https://<namespace>.compat.objectstorage.<region>.oraclecloud.com`, where the namespace is shown on the bucket's page. Fill in the `LITESTREAM_*` lines.

## First start

```sh
git clone https://github.com/nuzkyapp/nuzky.git && cd nuzky/account/deploy
cp .env.example .env && chmod 600 .env
openssl rand -hex 32   # paste as AUTH_SECRET
$EDITOR .env
docker compose up -d --build
docker compose logs -f account
```

The server refuses to start in production with a missing or short secret, an `http` base URL, the file mail transport or a Google discovery address other than Google's, and says which. It creates the tables and the `nuzky-desktop` client by itself.

Check it from your computer: `https://account.nuzky.app/api/auth/.well-known/openid-configuration` answers JSON, and `https://account.nuzky.app/sign-in` shows the sign-in page.

## Updating

```sh
cd nuzky && git pull && cd account/deploy && docker compose up -d --build --force-recreate
```

`--force-recreate` starts Caddy and Litestream anew too. Without it they keep the `Caddyfile` and `litestream.yml` they started with: a single file mounted into a container stays the one from before the pull. Every new container starts with an empty log.

Migrations run at start. The `AUTH_SECRET` must stay the same: it encrypts the signing keys and Google's tokens, and a new one signs everyone out of the browser and the app.

## Restoring a backup

Stop the server so nothing writes to the database, restore the newest copy into the volume, and start again:

```sh
cd nuzky/account/deploy
docker compose stop account litestream
docker compose run --rm --entrypoint sh litestream -c \
  'mv /data/account.db /data/account.db.broken 2>/dev/null; rm -f /data/account.db-wal /data/account.db-shm; litestream restore -config /etc/litestream.yml /data/account.db'
docker compose start account litestream
```

`litestream restore -timestamp 2026-11-14T12:00:00Z …` restores the database as it was at that moment instead. `litestream ltx -config /etc/litestream.yml /data/account.db` lists what the bucket holds.

## What runs where

- `Caddyfile`: TLS for `account.nuzky.app`, HSTS, compression and an access log on stdout without query strings, redirect targets, cookies, `Authorization` or `Referer`, since those carry codes and tokens. Requests to `/api/telemetry*` are not logged at all.
- `compose.yml`: the account server runs as an unprivileged user on a read-only filesystem; only `/data`, `/tmp` and Next's cache are writable. Litestream runs as the same user on the same volume. Docker keeps at most 3 × 10 MB of each container's log.
- `litestream.yml`: one replica of `account.db`; `telemetry.db` is not backed up, `s3` with path-style addresses and signed payloads for OCI's S3 API. A restore from OCI into a separate file was checked on the running server; the in-place restore steps above were tried with a local file replica. `docker compose exec litestream litestream ltx -config /etc/litestream.yml /data/account.db` lists what the bucket holds.
