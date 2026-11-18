# Native deployment

Use the [deployment guide](debian-deploy-quickstart.md) for the current release flow. This older filename is kept for existing links; it does not select a deployment domain.

Build on the designated Linux builder, deploy reviewed release artifacts, keep ingress behind the configured reverse proxy, and use a dedicated ViceDen database and role. Do not open public application or database ports or share another service's database.
