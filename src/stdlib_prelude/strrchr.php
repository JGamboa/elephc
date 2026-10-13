<?php
function strrchr(string $haystack, string $needle) {
    $c = $needle === "" ? "\0" : $needle[0];
    $pos = strrpos($haystack, $c);
    if ($pos === false) {
        return false;
    }
    return substr($haystack, $pos);
}
