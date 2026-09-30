# QBO Auth Broker

This stack deploys a minimal OAuth broker for the `qbo` CLI:

- API Gateway HTTP API
- API Gateway custom domain at `qbo-auth.mojility.ca`
- Python Lambda callback handler
- DynamoDB session table with TTL
- Secrets Manager secret for Intuit production client credentials
- ACM certificate with Route 53 DNS validation

The broker does not call QuickBooks business APIs. It only handles OAuth code exchange
and refresh-token exchange so the production Intuit client secret stays out of the CLI.

## Package

The account-specific, non-secret deployment parameters are stored in:

```text
infra/qbo-auth-broker/deploy.env
```

For routine updates, use the deploy script:

```sh
infra/qbo-auth-broker/deploy.sh
```

That script packages the Lambda, uploads it to the configured S3 bucket, deploys
CloudFormation, and prints the stack outputs.

Manual packaging is also possible:

```sh
mkdir -p infra/qbo-auth-broker/dist
cd infra/qbo-auth-broker/lambda
zip -r ../dist/qbo-auth-broker.zip .
```

Upload the zip to an S3 bucket you control:

```sh
aws s3 cp infra/qbo-auth-broker/dist/qbo-auth-broker.zip \
  s3://YOUR_DEPLOY_BUCKET/qbo-auth-broker.zip
```

## Deploy

For routine deploys, prefer:

```sh
infra/qbo-auth-broker/deploy.sh
```

The script uses these remembered parameters:

```text
AWS_REGION=ca-central-1
STACK_NAME=qbo-auth-broker
AUTH_DOMAIN_NAME=qbo-auth.mojility.ca
EXISTING_HOSTED_ZONE_ID=Z014134239FG2FBP7PBL9
LAMBDA_CODE_S3_BUCKET=cf-templates-1otvwja3sjodp-ca-central-1
LAMBDA_CODE_S3_KEY=qbo-auth-broker.zip
```

Manual deploys can use the same values.

Get the existing Route 53 hosted zone ID for `mojility.ca`:

```sh
aws route53 list-hosted-zones-by-name \
  --dns-name mojility.ca \
  --query "HostedZones[?Name=='mojility.ca.'].Id | [0]" \
  --output text
```

The stack does not create or replace the hosted zone. It only adds the ACM DNS
validation record and the `qbo-auth.mojility.ca` alias record inside that
existing zone.

The output may look like `/hostedzone/Z123...`; pass only the `Z123...` portion
to CloudFormation.

```sh
aws cloudformation deploy \
  --stack-name qbo-auth-broker \
  --template-file infra/qbo-auth-broker/template.yaml \
  --region ca-central-1 \
  --capabilities CAPABILITY_IAM \
  --parameter-overrides \
    ExistingHostedZoneId=Z014134239FG2FBP7PBL9 \
    AuthDomainName=qbo-auth.mojility.ca \
    LambdaCodeS3Bucket=cf-templates-1otvwja3sjodp-ca-central-1 \
    LambdaCodeS3Key=qbo-auth-broker.zip
```

## Configure Intuit Credentials

Get the secret name:

```sh
aws cloudformation describe-stacks \
  --stack-name qbo-auth-broker \
  --query "Stacks[0].Outputs[?OutputKey=='ClientCredentialsSecretName'].OutputValue" \
  --output text
```

Update it with the Intuit production client ID and client secret:

```sh
aws secretsmanager put-secret-value \
  --secret-id SECRET_NAME_FROM_OUTPUT \
  --secret-string '{"client_id":"YOUR_INTUIT_CLIENT_ID","client_secret":"YOUR_INTUIT_CLIENT_SECRET"}'
```

## Register Redirect URI

Get the production redirect URI. With the default parameters it will be
`https://qbo-auth.mojility.ca/callback`.

```sh
aws cloudformation describe-stacks \
  --stack-name qbo-auth-broker \
  --query "Stacks[0].Outputs[?OutputKey=='IntuitRedirectUri'].OutputValue" \
  --output text
```

Register that exact URI in Intuit Developer:

```text
Settings -> Redirect URIs -> Production
```

## Use From CLI

Get the broker base URL. With the default parameters it will be
`https://qbo-auth.mojility.ca`.

```sh
aws cloudformation describe-stacks \
  --stack-name qbo-auth-broker \
  --query "Stacks[0].Outputs[?OutputKey=='BrokerBaseUrl'].OutputValue" \
  --output text
```

Then authenticate:

```sh
cargo run -- auth login --broker-url BROKER_BASE_URL --environment production
cargo run -- customer list
```
