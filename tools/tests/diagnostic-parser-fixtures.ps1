Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../diagnostics/phone-parsers.psm1')
$fixture = @'
add device 1: /dev/input/event-private-touch
  name: "private-touch-name"
  events:
    ABS_MT_PRESSURE : value 0, min 0, max 255
    ABS_TILT_X : value 0, min -90, max 90
add device 2: /dev/input/event-private-pen
  name: "private-pen-name"
  events:
    BTN_TOOL_PEN
    ABS_PRESSURE : value 0, min 0, max 4095
    ABS_DISTANCE : value 0, min 0, max 255
    ABS_TILT_Y : value 0, min -90, max 90
'@
$pen = ConvertFrom-VwPenListing $fixture
$display = ConvertFrom-VwDisplayListing 'private-device supportedModes: [{width=1080, height=2400, fps=90.0}, {width=1080, height=2400, fps=90.0}, {width=1080, height=2400, fps=60.0}]'
$codec = ConvertFrom-VwCodecListing '<MediaCodec name="OMX.qcom.video.decoder.avc"/><MediaCodec name="OMX.qcom.video.decoder.avc"/><MediaCodec name="private-account"/><MediaCodec name="c2.android.aac.decoder"/>'
[ordered]@{
    pen = $pen; touch_only = ConvertFrom-VwPenListing 'add device 1: private-name ABS_MT_PRESSURE min 0, max 255'
    display = $display; codec = $codec
} | ConvertTo-Json -Depth 12
