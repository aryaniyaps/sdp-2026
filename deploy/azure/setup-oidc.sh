#!/usr/bin/env bash
# One-time setup, run by someone who can create app registrations in the Azure tenant and assign
# roles on the VM. It lets the GitHub workflow sign in without a stored secret (OIDC) and gives it
# the right to run commands on this one VM and nothing else. Prints the three values to store as
# secrets of the `azure` environment of the GitHub repository.
#   GITHUB_REPO  owner/name (default aryaniyaps/sdp-2026)
set -euo pipefail

RG=${AZURE_RG:-sdp-memory-rg}
VM=${AZURE_VM:-sdp-memory-vm}
GITHUB_REPO=${GITHUB_REPO:-aryaniyaps/sdp-2026}
NAME=sdp-memory-github-deploy

app=$(az ad app list --display-name "$NAME" --query '[0].appId' -o tsv)
if [ -z "$app" ]; then app=$(az ad app create --display-name "$NAME" --query appId -o tsv); fi
az ad sp show --id "$app" > /dev/null 2>&1 || az ad sp create --id "$app" > /dev/null

subject="repo:$GITHUB_REPO:environment:azure"
if ! az ad app federated-credential list --id "$app" --query "[?subject=='$subject'].name" -o tsv | grep -q .; then
  az ad app federated-credential create --id "$app" --parameters "{
    \"name\": \"github-azure-environment\",
    \"issuer\": \"https://token.actions.githubusercontent.com\",
    \"subject\": \"$subject\",
    \"audiences\": [\"api://AzureADTokenExchange\"]}" > /dev/null
fi

scope=$(az vm show -g "$RG" -n "$VM" --query id -o tsv)
az role assignment create --assignee "$app" --role "Virtual Machine Contributor" --scope "$scope" > /dev/null

echo "Store these as secrets of the 'azure' environment in $GITHUB_REPO (Settings, Environments):"
echo "  AZURE_CLIENT_ID=$app"
echo "  AZURE_TENANT_ID=$(az account show --query tenantId -o tsv)"
echo "  AZURE_SUBSCRIPTION_ID=$(az account show --query id -o tsv)"
