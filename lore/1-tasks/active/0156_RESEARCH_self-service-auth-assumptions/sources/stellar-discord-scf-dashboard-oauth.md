---
url: https://communityfund.stellar.org/dashboard
title: "SCF Dashboard sign-in — Discord OAuth redirect chain and application identity"
fetched_date: 2026-08-10
---

Evidence that a first-party SDF service authenticates solely via Discord OAuth,
and that it requests the broad `guilds` scope.

## Redirect chain (unauthenticated, `curl -L`)

```
HTTP/2 308 
location: /dashboard/award-rounds
HTTP/2 103 
HTTP/2 307 
location: /dashboard/login?callbackUrl=%2Fdashboard%2Faward-rounds
HTTP/2 307 
location: https://discord.com/api/oauth2/authorize?scope=identify+email+connections+guilds&response_type=code&client_id=917408694822658160&redirect_uri=https%3A%2F%2Fcommunityfund.stellar.org%2Fapi%2Fauth%2Fcallback%2Fdiscord
HTTP/2 302 
location: https://discord.com/oauth2/authorize?scope=identify+email+connections+guilds&response_type=code&client_id=917408694822658160&redirect_uri=https%3A%2F%2Fcommunityfund.stellar.org%2Fapi%2Fauth%2Fcallback%2Fdiscord
HTTP/2 200 
```

Requested scopes: `identify email connections guilds`
OAuth client_id: `917408694822658160`
Redirect URI: `https://communityfund.stellar.org/api/auth/callback/discord`

## Application identity

`GET https://discord.com/api/v10/applications/917408694822658160/rpc` (public):

```json
{
    "id": "917408694822658160",
    "name": "Stellar Community Fund",
    "icon": "578d0b2c76d33737930986bf00a905a8",
    "description": "Build, Engage, Launch, and Grow.",
    "summary": "",
    "type": null,
    "is_monetized": false,
    "is_verified": false,
    "is_discoverable": false,
    "hook": true,
    "storefront_available": false,
    "bot_public": false,
    "bot_require_code_grant": false,
    "terms_of_service_url": "https://www.stellar.org/terms-of-service",
    "privacy_policy_url": "https://www.stellar.org/privacy-policy",
    "integration_types_config": {
        "0": {}
    },
    "verify_key": "5f4e9f5cbc0fa1ebdab24f59462ccab2148963ab0c3af3a346e738b666954d05",
    "flags": 0,
    "flags_new": "0"
}
```
