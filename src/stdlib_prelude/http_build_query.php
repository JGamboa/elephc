<?php
function __elephc_hbq_enc(string $s, int $enc) {
    if ($enc === 2) { return rawurlencode($s); }
    return urlencode($s);
}
function __elephc_hbq_walk($data, $prefix, $numericPrefix, $enc, &$parts) {
    $items = [];
    if (is_object($data)) {
        $items = (array) $data;
    } else {
        $items = $data;
    }
    foreach ($items as $k => $v) {
        if ($v === null) { continue; }
        if (is_string($k) && substr($k, 0, 1) === "\0") { continue; }
        if (is_int($k)) {
            $name = $prefix === "" ? $numericPrefix . $k : $prefix . "%5B" . $k . "%5D";
        } else {
            $ek = __elephc_hbq_enc((string) $k, $enc);
            $name = $prefix === "" ? $ek : $prefix . "%5B" . $ek . "%5D";
        }
        if (is_array($v) || is_object($v)) {
            __elephc_hbq_walk($v, $name, $numericPrefix, $enc, $parts);
            continue;
        }
        if (is_bool($v)) {
            $s = $v ? "1" : "0";
        } elseif (is_float($v)) {
            $s = json_encode($v);
            if (strlen($s) > 2 && substr($s, -2) === ".0") { $s = substr($s, 0, -2); }
        } else {
            $s = (string) $v;
        }
        $parts[] = $name . "=" . __elephc_hbq_enc($s, $enc);
    }
}
function http_build_query($data, string $numeric_prefix = "", ?string $arg_separator = null, int $encoding_type = PHP_QUERY_RFC1738) {
    $parts = [];
    __elephc_hbq_walk($data, "", $numeric_prefix, $encoding_type, $parts);
    $sep = $arg_separator === null ? "&" : $arg_separator;
    return implode($sep, $parts);
}
