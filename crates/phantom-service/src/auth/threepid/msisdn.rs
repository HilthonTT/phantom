//! Phone numbers as the `msisdn` medium stores them: the E.164 digits with no
//! `+`, as other homeservers and identity servers do.

use phantom_core::{Err, Result};

/// E.164 numbers have at most fifteen digits; eight is a floor that no real
/// number with its country code falls below.
const DIGITS: std::ops::RangeInclusive<usize> = 8..=15;

/// Turns what a client sends, an ISO 3166-1 alpha-2 country and the number as
/// the user typed it, into an msisdn.
///
/// A number written internationally (a leading `+` or `00`) keeps its own
/// country code and the country is not consulted. Otherwise the country's
/// calling code is prefixed and one national trunk `0` dropped.
pub fn canonicalize_msisdn(country: &str, phone_number: &str) -> Result<String> {
    let typed = phone_number.trim();
    let digits: String = typed.chars().filter(char::is_ascii_digit).collect();

    if typed
        .chars()
        .any(|c| !(c.is_ascii_digit() || c.is_whitespace() || "+-().".contains(c)))
    {
        return Err!(Request(InvalidParam(
            "The phone number has characters a number cannot"
        )));
    }

    let msisdn = if typed.starts_with('+') {
        digits
    } else if let Some(international) = digits.strip_prefix("00") {
        international.to_owned()
    } else {
        let Some(code) = calling_code(country) else {
            return Err!(Request(InvalidParam("Unknown country code {country:?}")));
        };

        let national = digits.strip_prefix('0').unwrap_or(&digits);
        format!("{code}{national}")
    };

    if !DIGITS.contains(&msisdn.len()) || msisdn.starts_with('0') {
        return Err!(Request(InvalidParam("That is not a valid phone number")));
    }

    Ok(msisdn)
}

/// The ITU calling code for an ISO 3166-1 alpha-2 country, in any case.
#[must_use]
pub fn calling_code(country: &str) -> Option<&'static str> {
    let country = country.trim().to_ascii_uppercase();

    CALLING_CODES
        .binary_search_by(|(iso, _)| (*iso).cmp(country.as_str()))
        .ok()
        .map(|index| CALLING_CODES[index].1)
}

/// Sorted by country, for `calling_code`'s binary search.
static CALLING_CODES: &[(&str, &str)] = &[
    ("AD", "376"),
    ("AE", "971"),
    ("AF", "93"),
    ("AG", "1"),
    ("AI", "1"),
    ("AL", "355"),
    ("AM", "374"),
    ("AO", "244"),
    ("AR", "54"),
    ("AS", "1"),
    ("AT", "43"),
    ("AU", "61"),
    ("AW", "297"),
    ("AX", "358"),
    ("AZ", "994"),
    ("BA", "387"),
    ("BB", "1"),
    ("BD", "880"),
    ("BE", "32"),
    ("BF", "226"),
    ("BG", "359"),
    ("BH", "973"),
    ("BI", "257"),
    ("BJ", "229"),
    ("BL", "590"),
    ("BM", "1"),
    ("BN", "673"),
    ("BO", "591"),
    ("BQ", "599"),
    ("BR", "55"),
    ("BS", "1"),
    ("BT", "975"),
    ("BW", "267"),
    ("BY", "375"),
    ("BZ", "501"),
    ("CA", "1"),
    ("CC", "61"),
    ("CD", "243"),
    ("CF", "236"),
    ("CG", "242"),
    ("CH", "41"),
    ("CI", "225"),
    ("CK", "682"),
    ("CL", "56"),
    ("CM", "237"),
    ("CN", "86"),
    ("CO", "57"),
    ("CR", "506"),
    ("CU", "53"),
    ("CV", "238"),
    ("CW", "599"),
    ("CX", "61"),
    ("CY", "357"),
    ("CZ", "420"),
    ("DE", "49"),
    ("DJ", "253"),
    ("DK", "45"),
    ("DM", "1"),
    ("DO", "1"),
    ("DZ", "213"),
    ("EC", "593"),
    ("EE", "372"),
    ("EG", "20"),
    ("EH", "212"),
    ("ER", "291"),
    ("ES", "34"),
    ("ET", "251"),
    ("FI", "358"),
    ("FJ", "679"),
    ("FK", "500"),
    ("FM", "691"),
    ("FO", "298"),
    ("FR", "33"),
    ("GA", "241"),
    ("GB", "44"),
    ("GD", "1"),
    ("GE", "995"),
    ("GF", "594"),
    ("GG", "44"),
    ("GH", "233"),
    ("GI", "350"),
    ("GL", "299"),
    ("GM", "220"),
    ("GN", "224"),
    ("GP", "590"),
    ("GQ", "240"),
    ("GR", "30"),
    ("GT", "502"),
    ("GU", "1"),
    ("GW", "245"),
    ("GY", "592"),
    ("HK", "852"),
    ("HN", "504"),
    ("HR", "385"),
    ("HT", "509"),
    ("HU", "36"),
    ("ID", "62"),
    ("IE", "353"),
    ("IL", "972"),
    ("IM", "44"),
    ("IN", "91"),
    ("IO", "246"),
    ("IQ", "964"),
    ("IR", "98"),
    ("IS", "354"),
    ("IT", "39"),
    ("JE", "44"),
    ("JM", "1"),
    ("JO", "962"),
    ("JP", "81"),
    ("KE", "254"),
    ("KG", "996"),
    ("KH", "855"),
    ("KI", "686"),
    ("KM", "269"),
    ("KN", "1"),
    ("KP", "850"),
    ("KR", "82"),
    ("KW", "965"),
    ("KY", "1"),
    ("KZ", "7"),
    ("LA", "856"),
    ("LB", "961"),
    ("LC", "1"),
    ("LI", "423"),
    ("LK", "94"),
    ("LR", "231"),
    ("LS", "266"),
    ("LT", "370"),
    ("LU", "352"),
    ("LV", "371"),
    ("LY", "218"),
    ("MA", "212"),
    ("MC", "377"),
    ("MD", "373"),
    ("ME", "382"),
    ("MF", "590"),
    ("MG", "261"),
    ("MH", "692"),
    ("MK", "389"),
    ("ML", "223"),
    ("MM", "95"),
    ("MN", "976"),
    ("MO", "853"),
    ("MP", "1"),
    ("MQ", "596"),
    ("MR", "222"),
    ("MS", "1"),
    ("MT", "356"),
    ("MU", "230"),
    ("MV", "960"),
    ("MW", "265"),
    ("MX", "52"),
    ("MY", "60"),
    ("MZ", "258"),
    ("NA", "264"),
    ("NC", "687"),
    ("NE", "227"),
    ("NF", "672"),
    ("NG", "234"),
    ("NI", "505"),
    ("NL", "31"),
    ("NO", "47"),
    ("NP", "977"),
    ("NR", "674"),
    ("NU", "683"),
    ("NZ", "64"),
    ("OM", "968"),
    ("PA", "507"),
    ("PE", "51"),
    ("PF", "689"),
    ("PG", "675"),
    ("PH", "63"),
    ("PK", "92"),
    ("PL", "48"),
    ("PM", "508"),
    ("PR", "1"),
    ("PS", "970"),
    ("PT", "351"),
    ("PW", "680"),
    ("PY", "595"),
    ("QA", "974"),
    ("RE", "262"),
    ("RO", "40"),
    ("RS", "381"),
    ("RU", "7"),
    ("RW", "250"),
    ("SA", "966"),
    ("SB", "677"),
    ("SC", "248"),
    ("SD", "249"),
    ("SE", "46"),
    ("SG", "65"),
    ("SH", "290"),
    ("SI", "386"),
    ("SJ", "47"),
    ("SK", "421"),
    ("SL", "232"),
    ("SM", "378"),
    ("SN", "221"),
    ("SO", "252"),
    ("SR", "597"),
    ("SS", "211"),
    ("ST", "239"),
    ("SV", "503"),
    ("SX", "1"),
    ("SY", "963"),
    ("SZ", "268"),
    ("TC", "1"),
    ("TD", "235"),
    ("TG", "228"),
    ("TH", "66"),
    ("TJ", "992"),
    ("TK", "690"),
    ("TL", "670"),
    ("TM", "993"),
    ("TN", "216"),
    ("TO", "676"),
    ("TR", "90"),
    ("TT", "1"),
    ("TV", "688"),
    ("TW", "886"),
    ("TZ", "255"),
    ("UA", "380"),
    ("UG", "256"),
    ("US", "1"),
    ("UY", "598"),
    ("UZ", "998"),
    ("VA", "39"),
    ("VC", "1"),
    ("VE", "58"),
    ("VG", "1"),
    ("VI", "1"),
    ("VN", "84"),
    ("VU", "678"),
    ("WF", "681"),
    ("WS", "685"),
    ("XK", "383"),
    ("YE", "967"),
    ("YT", "262"),
    ("ZA", "27"),
    ("ZM", "260"),
    ("ZW", "263"),
];

#[cfg(test)]
mod tests {
    use super::{CALLING_CODES, calling_code, canonicalize_msisdn};

    #[test]
    fn the_table_is_sorted_for_binary_search() {
        assert!(CALLING_CODES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn a_national_number_takes_the_country_code() {
        assert_eq!(
            canonicalize_msisdn("GB", "07700 900123").unwrap(),
            "447700900123"
        );
        assert_eq!(
            canonicalize_msisdn("us", "(415) 555-0100").unwrap(),
            "14155550100"
        );
        assert_eq!(
            canonicalize_msisdn("BE", "0470 12 34 56").unwrap(),
            "32470123456"
        );
    }

    #[test]
    fn an_international_number_keeps_its_own_code() {
        assert_eq!(
            canonicalize_msisdn("US", "+44 7700 900123").unwrap(),
            "447700900123"
        );
        assert_eq!(
            canonicalize_msisdn("FR", "0044 7700 900123").unwrap(),
            "447700900123"
        );
    }

    #[test]
    fn nonsense_is_refused() {
        assert!(canonicalize_msisdn("GB", "call me").is_err());
        assert!(canonicalize_msisdn("GB", "123").is_err());
        assert!(canonicalize_msisdn("ZZ", "07700 900123").is_err());
        assert!(canonicalize_msisdn("US", "+1 234 567 890 123 456").is_err());
    }

    #[test]
    fn country_lookup_ignores_case() {
        assert_eq!(calling_code("de"), Some("49"));
        assert_eq!(calling_code("nowhere"), None);
    }
}
