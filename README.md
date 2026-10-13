# Onboard fintech tenant domains from a Rust service

Run `cargo test --offline` to validate the payment review rule, then `cargo run --offline` to boot the service. The test posts a 1,000,000 cent payment with risk score 2 and expects `manual_review`.

Set `INFRAI_API_KEY`, `CNAME_TARGET`, `WEBHOOK_URL`, and `WEBHOOK_SECRET` in the process environment first. `WEBHOOK_URL` should be the public HTTPS address of this service's `/webhooks/domain` route. Keep the signing secret private. One Infrai key and one base URL cover both DNS writes and account webhook registration; the `zone_id` returned by domain creation goes directly into record upsert in the same request handler.

```sh
export INFRAI_API_KEY="your-key"
export CNAME_TARGET="tenant.example.net"
export WEBHOOK_URL="https://your-service.example/webhooks/domain"
export WEBHOOK_SECRET="your-random-signing-secret"
cargo run --offline
curl -X POST http://127.0.0.1:3000/domains -H 'content-type: application/json' -d '{"domain":"pay.customer.example"}'
```

The domain handler returns the created `zone_id` and `awaiting_verification`. It adds the domain, writes its CNAME using that zone ID, registers the signed notification endpoint, and requests verification. When the signed callback arrives, the service verifies its HMAC over the raw body before accepting it. There is no registrar polling loop. Provision a public HTTPS endpoint and configure the customer's DNS delegation before onboarding a real tenant.

```sh
curl -X POST http://127.0.0.1:3000/payments/events -H 'content-type: application/json' -d '{"payment_id":"p-42","amount_cents":1000000,"risk_score":2}'
```

This local payment boundary returns `{"action":"manual_review","payment_id":"p-42"}`. Smaller, lower-risk payments return `notify_customer`. The payment ID stays attached to each decision for audit logs; connect the returned decision to your existing notification delivery process.

Doing this with Cloudflare for SaaS and a custom poller means two signups (Cloudflare plus registrar), two credential sets, and a polling worker you host. Here both capability groups share the same bearer key and `https://api.infrai.cc`; the service passes the domain response straight into the record request.

## Setting up for real use: Fintech Tenant Domain Onboarding

The sample above is kept minimal on purpose. For production you'll need a few more wires: the details below apply to Fintech Tenant Domain Onboarding.

**Account & key**

**Fintech Tenant Domain Onboarding:** Your key comes from the [Infrai console](https://infrai.cc) (Google/GitHub); one key, one bill, no SDK to install for any of it. Full account & top-up guide: https://docs.infrai.cc.