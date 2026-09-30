#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_FILE="${1:-"${SCRIPT_DIR}/deploy.env"}"

if [[ ! -f "${CONFIG_FILE}" ]]; then
  echo "Missing deploy config: ${CONFIG_FILE}" >&2
  exit 1
fi

set -a
source "${CONFIG_FILE}"
set +a

: "${AWS_REGION:?AWS_REGION is required}"
: "${STACK_NAME:?STACK_NAME is required}"
: "${AUTH_DOMAIN_NAME:?AUTH_DOMAIN_NAME is required}"
: "${EXISTING_HOSTED_ZONE_ID:?EXISTING_HOSTED_ZONE_ID is required}"
: "${LAMBDA_CODE_S3_BUCKET:?LAMBDA_CODE_S3_BUCKET is required}"
: "${LAMBDA_CODE_S3_KEY:?LAMBDA_CODE_S3_KEY is required}"

DIST_DIR="${SCRIPT_DIR}/dist"
ZIP_PATH="${DIST_DIR}/qbo-auth-broker.zip"

mkdir -p "${DIST_DIR}"
rm -f "${ZIP_PATH}"

(
  cd "${SCRIPT_DIR}/lambda"
  zip -qr "${ZIP_PATH}" .
)

aws s3 cp "${ZIP_PATH}" "s3://${LAMBDA_CODE_S3_BUCKET}/${LAMBDA_CODE_S3_KEY}" \
  --region "${AWS_REGION}"

aws cloudformation deploy \
  --region "${AWS_REGION}" \
  --stack-name "${STACK_NAME}" \
  --template-file "${SCRIPT_DIR}/template.yaml" \
  --capabilities CAPABILITY_IAM \
  --parameter-overrides \
    ExistingHostedZoneId="${EXISTING_HOSTED_ZONE_ID}" \
    AuthDomainName="${AUTH_DOMAIN_NAME}" \
    LambdaCodeS3Bucket="${LAMBDA_CODE_S3_BUCKET}" \
    LambdaCodeS3Key="${LAMBDA_CODE_S3_KEY}"

aws cloudformation describe-stacks \
  --region "${AWS_REGION}" \
  --stack-name "${STACK_NAME}" \
  --query "Stacks[0].Outputs" \
  --output table
