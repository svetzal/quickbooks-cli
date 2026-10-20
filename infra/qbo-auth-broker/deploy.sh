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
QBO_ENVIRONMENT="${QBO_ENVIRONMENT:-sandbox}"

# The broker and its session data are hosted only in Canada.
if [[ "${AWS_REGION}" != "ca-central-1" ]]; then
  echo "AWS_REGION must be ca-central-1 for the QBO auth broker." >&2
  exit 1
fi
if [[ "${QBO_ENVIRONMENT}" != "sandbox" && "${QBO_ENVIRONMENT}" != "production" ]]; then
  echo "QBO_ENVIRONMENT must be sandbox or production." >&2
  exit 1
fi

DIST_DIR="${SCRIPT_DIR}/dist"
ZIP_PATH="${DIST_DIR}/qbo-auth-broker.zip"

"${SCRIPT_DIR}/package.sh"

aws s3 cp "${ZIP_PATH}" "s3://${LAMBDA_CODE_S3_BUCKET}/${LAMBDA_CODE_S3_KEY}" \
  --region "${AWS_REGION}"

aws cloudformation deploy \
  --region "${AWS_REGION}" \
  --stack-name "${STACK_NAME}" \
  --template-file "${SCRIPT_DIR}/template.yaml" \
  --capabilities CAPABILITY_IAM \
  --parameter-overrides \
    QboEnvironment="${QBO_ENVIRONMENT}" \
    ExistingHostedZoneId="${EXISTING_HOSTED_ZONE_ID}" \
    AuthDomainName="${AUTH_DOMAIN_NAME}" \
    LambdaCodeS3Bucket="${LAMBDA_CODE_S3_BUCKET}" \
    LambdaCodeS3Key="${LAMBDA_CODE_S3_KEY}"

aws cloudformation describe-stacks \
  --region "${AWS_REGION}" \
  --stack-name "${STACK_NAME}" \
  --query "Stacks[0].Outputs" \
  --output table
