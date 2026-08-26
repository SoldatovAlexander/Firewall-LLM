{{- define "fwllm.name" -}}{{ default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}{{- end }}
{{- define "fwllm.fullname" -}}{{ .Release.Name }}-{{ include "fwllm.name" . }}{{- end }}
{{- define "fwllm.labels" -}}helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version }}
app.kubernetes.io/name: {{ include "fwllm.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}{{- end }}
{{- define "fwllm.selectorLabels" -}}app.kubernetes.io/name: {{ include "fwllm.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}{{- end }}
