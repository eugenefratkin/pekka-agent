# Deploying to Railway

## 1. Create Google OAuth credentials

1. Go to [Google Cloud Console](https://console.cloud.google.com) → **APIs & Services** → **Credentials**
2. Click **Create Credentials** → **OAuth 2.0 Client ID**
3. Application type: **Web application**
4. Under **Authorized redirect URIs**, add:
   ```
   https://<your-railway-domain>/auth/callback
   ```
   (You'll get the domain after the first Railway deploy — come back and add it)
5. Copy the **Client ID** and **Client Secret**

## 2. Deploy on Railway

[![Deploy on Railway](https://railway.com/button.svg)](https://railway.com/new/template)

Or manually:
1. Create a new project in [Railway](https://railway.com)
2. Connect the `eugenefratkin/pekka-agent` GitHub repo
3. Railway auto-detects `railway.toml` and uses Nixpacks to build

## 3. Set environment variables in Railway

| Variable | Value |
|---|---|
| `GOOGLE_CLIENT_ID` | from Google Cloud Console |
| `GOOGLE_CLIENT_SECRET` | from Google Cloud Console |
| `BASE_URL` | `https://<your-railway-domain>` |
| `PERPLEXITY_API_KEY` | your Perplexity key from [perplexity.ai/settings/api](https://www.perplexity.ai/settings/api) |
| `WHITELISTED_EMAILS` | `eugenefratkin@gmail.com,efratkin@salesforce.com,shailesh.kumar@salesforce.com` |

> **Note:** `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` must both be set to enable login. Without them the app runs in open (no-auth) mode.

## 4. Update the OAuth redirect URI

Once Railway assigns a domain (e.g. `pekka-agent.up.railway.app`):

1. Go back to Google Cloud Console → your OAuth client
2. Add `https://pekka-agent.up.railway.app/auth/callback` to **Authorized redirect URIs**
3. Set `BASE_URL=https://pekka-agent.up.railway.app` in Railway env vars
4. Redeploy (or Railway will pick it up automatically)

## Access control

Anyone not in `WHITELISTED_EMAILS` who signs in with Google will see an **Access denied** page. To add someone, append their email to the `WHITELISTED_EMAILS` env var and redeploy.

## Running locally

```bash
# Install deps
npm install

# Start (port 3754)
npm start
# → http://localhost:3754

# With a different port
PORT=8080 node mock-server.js
```

Local mode skips Google login unless `GOOGLE_CLIENT_ID` is set in `.env`.
