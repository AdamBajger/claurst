{{/*
Expand the chart name.
*/}}
{{- define "claurst.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully-qualified app name.
Truncate at 63 chars because Kubernetes DNS name limits.
*/}}
{{- define "claurst.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart label.
*/}}
{{- define "claurst.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels.
*/}}
{{- define "claurst.labels" -}}
helm.sh/chart: {{ include "claurst.chart" . }}
{{ include "claurst.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels.
*/}}
{{- define "claurst.selectorLabels" -}}
app.kubernetes.io/name: {{ include "claurst.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Service account name.
*/}}
{{- define "claurst.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "claurst.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Container image reference.
*/}}
{{- define "claurst.image" -}}
{{- $tag := .Values.image.tag | default .Chart.AppVersion }}
{{- printf "%s:%s" .Values.image.repository $tag }}
{{- end }}

{{/*
Secret name for SSH keys and API credentials.
*/}}
{{- define "claurst.secretName" -}}
{{- printf "%s-secrets" (include "claurst.fullname" .) }}
{{- end }}

{{/*
ConfigMap name for extra environment variables.
*/}}
{{- define "claurst.configMapName" -}}
{{- printf "%s-env" (include "claurst.fullname" .) }}
{{- end }}

{{/*
PVC name for SSH host key persistence.
*/}}
{{- define "claurst.pvcName" -}}
{{- if .Values.persistence.existingClaim }}
{{- .Values.persistence.existingClaim }}
{{- else }}
{{- printf "%s-ssh-hostkeys" (include "claurst.fullname" .) }}
{{- end }}
{{- end }}
