//! Support for the **Polyglot** format (`.bin`) for opening books.
//!
//! Polyglot is an open, publicly documented format (Fabien Letouzey),
//! used as a reference by many chess programs (Arena, Scid,
//! Cute Chess, `ChessBase`...). A book is a sequence of 16-byte entries,
//! sorted by increasing hash key:
//!
//! | Bytes  | Content                                            |
//! |--------|-----------------------------------------------------|
//! | 0-7    | Zobrist hash key (`u64`, big-endian)                 |
//! | 8-9    | Encoded move (`u16`, big-endian)                     |
//! | 10-11  | Weight (`u16`, big-endian)                           |
//! | 12-15  | Learning value (`u32`, big-endian, ignored)          |
//!
//! Deliberately supported: Polyglot only. `ChessBase` CTG and
//! Arena ABK are proprietary formats never officially documented
//! and are not implemented (see `Analyse_Projet/SUIVI_PLAN_ACTION.md`,
//! PHASE 15).
//!
//! Architecture: it is the **GUI** that reads the book and chooses the move
//! (like Arena, Scid, Cute Chess), not the UCI engine — this allows
//! a book to work with any external engine, with no
//! extra configuration on the user's side.

use crate::types::{Color, Piece, PieceKind, Position, Square};
use std::time::{SystemTime, UNIX_EPOCH};

/// Table of the 781 pseudo-random constants for the Polyglot Zobrist hash.
///
/// Canonical values of the format (identical across all implementations:
/// Polyglot itself, python-chess, Cute Chess, Scid...). Copied as-is
/// from the `python-chess` reference
/// (<https://github.com/niklasf/python-chess/blob/master/chess/polyglot.py>)
/// to avoid any risk of manual transcription error — a single
/// wrong value would produce a hash that would silently never match a
/// real `.bin` file.
///
/// Clippy (04/07/2026, post-audit fixes): deliberate `#[allow(unreadable_literal)]`
/// on these 781 constants — a frozen table copied verbatim from an
/// authoritative external reference; reformatting (adding `_` every
/// 4 digits) would bring no real readability benefit and would risk
/// silently corrupting a value during retranscription.
#[rustfmt::skip]
#[allow(clippy::unreadable_literal)]
const RANDOM_ARRAY: [u64; 781] = [
    0x9D39247E33776D41,    0x2AF7398005AAA5C7,    0x44DB015024623547,    0x9C15F73E62A76AE2,
    0x75834465489C0C89,    0x3290AC3A203001BF,    0x0FBBAD1F61042279,    0xE83A908FF2FB60CA,
    0x0D7E765D58755C10,    0x1A083822CEAFE02D,    0x9605D5F0E25EC3B0,    0xD021FF5CD13A2ED5,
    0x40BDF15D4A672E32,    0x011355146FD56395,    0x5DB4832046F3D9E5,    0x239F8B2D7FF719CC,
    0x05D1A1AE85B49AA1,    0x679F848F6E8FC971,    0x7449BBFF801FED0B,    0x7D11CDB1C3B7ADF0,
    0x82C7709E781EB7CC,    0xF3218F1C9510786C,    0x331478F3AF51BBE6,    0x4BB38DE5E7219443,
    0xAA649C6EBCFD50FC,    0x8DBD98A352AFD40B,    0x87D2074B81D79217,    0x19F3C751D3E92AE1,
    0xB4AB30F062B19ABF,    0x7B0500AC42047AC4,    0xC9452CA81A09D85D,    0x24AA6C514DA27500,
    0x4C9F34427501B447,    0x14A68FD73C910841,    0xA71B9B83461CBD93,    0x03488B95B0F1850F,
    0x637B2B34FF93C040,    0x09D1BC9A3DD90A94,    0x3575668334A1DD3B,    0x735E2B97A4C45A23,
    0x18727070F1BD400B,    0x1FCBACD259BF02E7,    0xD310A7C2CE9B6555,    0xBF983FE0FE5D8244,
    0x9F74D14F7454A824,    0x51EBDC4AB9BA3035,    0x5C82C505DB9AB0FA,    0xFCF7FE8A3430B241,
    0x3253A729B9BA3DDE,    0x8C74C368081B3075,    0xB9BC6C87167C33E7,    0x7EF48F2B83024E20,
    0x11D505D4C351BD7F,    0x6568FCA92C76A243,    0x4DE0B0F40F32A7B8,    0x96D693460CC37E5D,
    0x42E240CB63689F2F,    0x6D2BDCDAE2919661,    0x42880B0236E4D951,    0x5F0F4A5898171BB6,
    0x39F890F579F92F88,    0x93C5B5F47356388B,    0x63DC359D8D231B78,    0xEC16CA8AEA98AD76,
    0x5355F900C2A82DC7,    0x07FB9F855A997142,    0x5093417AA8A7ED5E,    0x7BCBC38DA25A7F3C,
    0x19FC8A768CF4B6D4,    0x637A7780DECFC0D9,    0x8249A47AEE0E41F7,    0x79AD695501E7D1E8,
    0x14ACBAF4777D5776,    0xF145B6BECCDEA195,    0xDABF2AC8201752FC,    0x24C3C94DF9C8D3F6,
    0xBB6E2924F03912EA,    0x0CE26C0B95C980D9,    0xA49CD132BFBF7CC4,    0xE99D662AF4243939,
    0x27E6AD7891165C3F,    0x8535F040B9744FF1,    0x54B3F4FA5F40D873,    0x72B12C32127FED2B,
    0xEE954D3C7B411F47,    0x9A85AC909A24EAA1,    0x70AC4CD9F04F21F5,    0xF9B89D3E99A075C2,
    0x87B3E2B2B5C907B1,    0xA366E5B8C54F48B8,    0xAE4A9346CC3F7CF2,    0x1920C04D47267BBD,
    0x87BF02C6B49E2AE9,    0x092237AC237F3859,    0xFF07F64EF8ED14D0,    0x8DE8DCA9F03CC54E,
    0x9C1633264DB49C89,    0xB3F22C3D0B0B38ED,    0x390E5FB44D01144B,    0x5BFEA5B4712768E9,
    0x1E1032911FA78984,    0x9A74ACB964E78CB3,    0x4F80F7A035DAFB04,    0x6304D09A0B3738C4,
    0x2171E64683023A08,    0x5B9B63EB9CEFF80C,    0x506AACF489889342,    0x1881AFC9A3A701D6,
    0x6503080440750644,    0xDFD395339CDBF4A7,    0xEF927DBCF00C20F2,    0x7B32F7D1E03680EC,
    0xB9FD7620E7316243,    0x05A7E8A57DB91B77,    0xB5889C6E15630A75,    0x4A750A09CE9573F7,
    0xCF464CEC899A2F8A,    0xF538639CE705B824,    0x3C79A0FF5580EF7F,    0xEDE6C87F8477609D,
    0x799E81F05BC93F31,    0x86536B8CF3428A8C,    0x97D7374C60087B73,    0xA246637CFF328532,
    0x043FCAE60CC0EBA0,    0x920E449535DD359E,    0x70EB093B15B290CC,    0x73A1921916591CBD,
    0x56436C9FE1A1AA8D,    0xEFAC4B70633B8F81,    0xBB215798D45DF7AF,    0x45F20042F24F1768,
    0x930F80F4E8EB7462,    0xFF6712FFCFD75EA1,    0xAE623FD67468AA70,    0xDD2C5BC84BC8D8FC,
    0x7EED120D54CF2DD9,    0x22FE545401165F1C,    0xC91800E98FB99929,    0x808BD68E6AC10365,
    0xDEC468145B7605F6,    0x1BEDE3A3AEF53302,    0x43539603D6C55602,    0xAA969B5C691CCB7A,
    0xA87832D392EFEE56,    0x65942C7B3C7E11AE,    0xDED2D633CAD004F6,    0x21F08570F420E565,
    0xB415938D7DA94E3C,    0x91B859E59ECB6350,    0x10CFF333E0ED804A,    0x28AED140BE0BB7DD,
    0xC5CC1D89724FA456,    0x5648F680F11A2741,    0x2D255069F0B7DAB3,    0x9BC5A38EF729ABD4,
    0xEF2F054308F6A2BC,    0xAF2042F5CC5C2858,    0x480412BAB7F5BE2A,    0xAEF3AF4A563DFE43,
    0x19AFE59AE451497F,    0x52593803DFF1E840,    0xF4F076E65F2CE6F0,    0x11379625747D5AF3,
    0xBCE5D2248682C115,    0x9DA4243DE836994F,    0x066F70B33FE09017,    0x4DC4DE189B671A1C,
    0x51039AB7712457C3,    0xC07A3F80C31FB4B4,    0xB46EE9C5E64A6E7C,    0xB3819A42ABE61C87,
    0x21A007933A522A20,    0x2DF16F761598AA4F,    0x763C4A1371B368FD,    0xF793C46702E086A0,
    0xD7288E012AEB8D31,    0xDE336A2A4BC1C44B,    0x0BF692B38D079F23,    0x2C604A7A177326B3,
    0x4850E73E03EB6064,    0xCFC447F1E53C8E1B,    0xB05CA3F564268D99,    0x9AE182C8BC9474E8,
    0xA4FC4BD4FC5558CA,    0xE755178D58FC4E76,    0x69B97DB1A4C03DFE,    0xF9B5B7C4ACC67C96,
    0xFC6A82D64B8655FB,    0x9C684CB6C4D24417,    0x8EC97D2917456ED0,    0x6703DF9D2924E97E,
    0xC547F57E42A7444E,    0x78E37644E7CAD29E,    0xFE9A44E9362F05FA,    0x08BD35CC38336615,
    0x9315E5EB3A129ACE,    0x94061B871E04DF75,    0xDF1D9F9D784BA010,    0x3BBA57B68871B59D,
    0xD2B7ADEEDED1F73F,    0xF7A255D83BC373F8,    0xD7F4F2448C0CEB81,    0xD95BE88CD210FFA7,
    0x336F52F8FF4728E7,    0xA74049DAC312AC71,    0xA2F61BB6E437FDB5,    0x4F2A5CB07F6A35B3,
    0x87D380BDA5BF7859,    0x16B9F7E06C453A21,    0x7BA2484C8A0FD54E,    0xF3A678CAD9A2E38C,
    0x39B0BF7DDE437BA2,    0xFCAF55C1BF8A4424,    0x18FCF680573FA594,    0x4C0563B89F495AC3,
    0x40E087931A00930D,    0x8CFFA9412EB642C1,    0x68CA39053261169F,    0x7A1EE967D27579E2,
    0x9D1D60E5076F5B6F,    0x3810E399B6F65BA2,    0x32095B6D4AB5F9B1,    0x35CAB62109DD038A,
    0xA90B24499FCFAFB1,    0x77A225A07CC2C6BD,    0x513E5E634C70E331,    0x4361C0CA3F692F12,
    0xD941ACA44B20A45B,    0x528F7C8602C5807B,    0x52AB92BEB9613989,    0x9D1DFA2EFC557F73,
    0x722FF175F572C348,    0x1D1260A51107FE97,    0x7A249A57EC0C9BA2,    0x04208FE9E8F7F2D6,
    0x5A110C6058B920A0,    0x0CD9A497658A5698,    0x56FD23C8F9715A4C,    0x284C847B9D887AAE,
    0x04FEABFBBDB619CB,    0x742E1E651C60BA83,    0x9A9632E65904AD3C,    0x881B82A13B51B9E2,
    0x506E6744CD974924,    0xB0183DB56FFC6A79,    0x0ED9B915C66ED37E,    0x5E11E86D5873D484,
    0xF678647E3519AC6E,    0x1B85D488D0F20CC5,    0xDAB9FE6525D89021,    0x0D151D86ADB73615,
    0xA865A54EDCC0F019,    0x93C42566AEF98FFB,    0x99E7AFEABE000731,    0x48CBFF086DDF285A,
    0x7F9B6AF1EBF78BAF,    0x58627E1A149BBA21,    0x2CD16E2ABD791E33,    0xD363EFF5F0977996,
    0x0CE2A38C344A6EED,    0x1A804AADB9CFA741,    0x907F30421D78C5DE,    0x501F65EDB3034D07,
    0x37624AE5A48FA6E9,    0x957BAF61700CFF4E,    0x3A6C27934E31188A,    0xD49503536ABCA345,
    0x088E049589C432E0,    0xF943AEE7FEBF21B8,    0x6C3B8E3E336139D3,    0x364F6FFA464EE52E,
    0xD60F6DCEDC314222,    0x56963B0DCA418FC0,    0x16F50EDF91E513AF,    0xEF1955914B609F93,
    0x565601C0364E3228,    0xECB53939887E8175,    0xBAC7A9A18531294B,    0xB344C470397BBA52,
    0x65D34954DAF3CEBD,    0xB4B81B3FA97511E2,    0xB422061193D6F6A7,    0x071582401C38434D,
    0x7A13F18BBEDC4FF5,    0xBC4097B116C524D2,    0x59B97885E2F2EA28,    0x99170A5DC3115544,
    0x6F423357E7C6A9F9,    0x325928EE6E6F8794,    0xD0E4366228B03343,    0x565C31F7DE89EA27,
    0x30F5611484119414,    0xD873DB391292ED4F,    0x7BD94E1D8E17DEBC,    0xC7D9F16864A76E94,
    0x947AE053EE56E63C,    0xC8C93882F9475F5F,    0x3A9BF55BA91F81CA,    0xD9A11FBB3D9808E4,
    0x0FD22063EDC29FCA,    0xB3F256D8ACA0B0B9,    0xB03031A8B4516E84,    0x35DD37D5871448AF,
    0xE9F6082B05542E4E,    0xEBFAFA33D7254B59,    0x9255ABB50D532280,    0xB9AB4CE57F2D34F3,
    0x693501D628297551,    0xC62C58F97DD949BF,    0xCD454F8F19C5126A,    0xBBE83F4ECC2BDECB,
    0xDC842B7E2819E230,    0xBA89142E007503B8,    0xA3BC941D0A5061CB,    0xE9F6760E32CD8021,
    0x09C7E552BC76492F,    0x852F54934DA55CC9,    0x8107FCCF064FCF56,    0x098954D51FFF6580,
    0x23B70EDB1955C4BF,    0xC330DE426430F69D,    0x4715ED43E8A45C0A,    0xA8D7E4DAB780A08D,
    0x0572B974F03CE0BB,    0xB57D2E985E1419C7,    0xE8D9ECBE2CF3D73F,    0x2FE4B17170E59750,
    0x11317BA87905E790,    0x7FBF21EC8A1F45EC,    0x1725CABFCB045B00,    0x964E915CD5E2B207,
    0x3E2B8BCBF016D66D,    0xBE7444E39328A0AC,    0xF85B2B4FBCDE44B7,    0x49353FEA39BA63B1,
    0x1DD01AAFCD53486A,    0x1FCA8A92FD719F85,    0xFC7C95D827357AFA,    0x18A6A990C8B35EBD,
    0xCCCB7005C6B9C28D,    0x3BDBB92C43B17F26,    0xAA70B5B4F89695A2,    0xE94C39A54A98307F,
    0xB7A0B174CFF6F36E,    0xD4DBA84729AF48AD,    0x2E18BC1AD9704A68,    0x2DE0966DAF2F8B1C,
    0xB9C11D5B1E43A07E,    0x64972D68DEE33360,    0x94628D38D0C20584,    0xDBC0D2B6AB90A559,
    0xD2733C4335C6A72F,    0x7E75D99D94A70F4D,    0x6CED1983376FA72B,    0x97FCAACBF030BC24,
    0x7B77497B32503B12,    0x8547EDDFB81CCB94,    0x79999CDFF70902CB,    0xCFFE1939438E9B24,
    0x829626E3892D95D7,    0x92FAE24291F2B3F1,    0x63E22C147B9C3403,    0xC678B6D860284A1C,
    0x5873888850659AE7,    0x0981DCD296A8736D,    0x9F65789A6509A440,    0x9FF38FED72E9052F,
    0xE479EE5B9930578C,    0xE7F28ECD2D49EECD,    0x56C074A581EA17FE,    0x5544F7D774B14AEF,
    0x7B3F0195FC6F290F,    0x12153635B2C0CF57,    0x7F5126DBBA5E0CA7,    0x7A76956C3EAFB413,
    0x3D5774A11D31AB39,    0x8A1B083821F40CB4,    0x7B4A38E32537DF62,    0x950113646D1D6E03,
    0x4DA8979A0041E8A9,    0x3BC36E078F7515D7,    0x5D0A12F27AD310D1,    0x7F9D1A2E1EBE1327,
    0xDA3A361B1C5157B1,    0xDCDD7D20903D0C25,    0x36833336D068F707,    0xCE68341F79893389,
    0xAB9090168DD05F34,    0x43954B3252DC25E5,    0xB438C2B67F98E5E9,    0x10DCD78E3851A492,
    0xDBC27AB5447822BF,    0x9B3CDB65F82CA382,    0xB67B7896167B4C84,    0xBFCED1B0048EAC50,
    0xA9119B60369FFEBD,    0x1FFF7AC80904BF45,    0xAC12FB171817EEE7,    0xAF08DA9177DDA93D,
    0x1B0CAB936E65C744,    0xB559EB1D04E5E932,    0xC37B45B3F8D6F2BA,    0xC3A9DC228CAAC9E9,
    0xF3B8B6675A6507FF,    0x9FC477DE4ED681DA,    0x67378D8ECCEF96CB,    0x6DD856D94D259236,
    0xA319CE15B0B4DB31,    0x073973751F12DD5E,    0x8A8E849EB32781A5,    0xE1925C71285279F5,
    0x74C04BF1790C0EFE,    0x4DDA48153C94938A,    0x9D266D6A1CC0542C,    0x7440FB816508C4FE,
    0x13328503DF48229F,    0xD6BF7BAEE43CAC40,    0x4838D65F6EF6748F,    0x1E152328F3318DEA,
    0x8F8419A348F296BF,    0x72C8834A5957B511,    0xD7A023A73260B45C,    0x94EBC8ABCFB56DAE,
    0x9FC10D0F989993E0,    0xDE68A2355B93CAE6,    0xA44CFE79AE538BBE,    0x9D1D84FCCE371425,
    0x51D2B1AB2DDFB636,    0x2FD7E4B9E72CD38C,    0x65CA5B96B7552210,    0xDD69A0D8AB3B546D,
    0x604D51B25FBF70E2,    0x73AA8A564FB7AC9E,    0x1A8C1E992B941148,    0xAAC40A2703D9BEA0,
    0x764DBEAE7FA4F3A6,    0x1E99B96E70A9BE8B,    0x2C5E9DEB57EF4743,    0x3A938FEE32D29981,
    0x26E6DB8FFDF5ADFE,    0x469356C504EC9F9D,    0xC8763C5B08D1908C,    0x3F6C6AF859D80055,
    0x7F7CC39420A3A545,    0x9BFB227EBDF4C5CE,    0x89039D79D6FC5C5C,    0x8FE88B57305E2AB6,
    0xA09E8C8C35AB96DE,    0xFA7E393983325753,    0xD6B6D0ECC617C699,    0xDFEA21EA9E7557E3,
    0xB67C1FA481680AF8,    0xCA1E3785A9E724E5,    0x1CFC8BED0D681639,    0xD18D8549D140CAEA,
    0x4ED0FE7E9DC91335,    0xE4DBF0634473F5D2,    0x1761F93A44D5AEFE,    0x53898E4C3910DA55,
    0x734DE8181F6EC39A,    0x2680B122BAA28D97,    0x298AF231C85BAFAB,    0x7983EED3740847D5,
    0x66C1A2A1A60CD889,    0x9E17E49642A3E4C1,    0xEDB454E7BADC0805,    0x50B704CAB602C329,
    0x4CC317FB9CDDD023,    0x66B4835D9EAFEA22,    0x219B97E26FFC81BD,    0x261E4E4C0A333A9D,
    0x1FE2CCA76517DB90,    0xD7504DFA8816EDBB,    0xB9571FA04DC089C8,    0x1DDC0325259B27DE,
    0xCF3F4688801EB9AA,    0xF4F5D05C10CAB243,    0x38B6525C21A42B0E,    0x36F60E2BA4FA6800,
    0xEB3593803173E0CE,    0x9C4CD6257C5A3603,    0xAF0C317D32ADAA8A,    0x258E5A80C7204C4B,
    0x8B889D624D44885D,    0xF4D14597E660F855,    0xD4347F66EC8941C3,    0xE699ED85B0DFB40D,
    0x2472F6207C2D0484,    0xC2A1E7B5B459AEB5,    0xAB4F6451CC1D45EC,    0x63767572AE3D6174,
    0xA59E0BD101731A28,    0x116D0016CB948F09,    0x2CF9C8CA052F6E9F,    0x0B090A7560A968E3,
    0xABEEDDB2DDE06FF1,    0x58EFC10B06A2068D,    0xC6E57A78FBD986E0,    0x2EAB8CA63CE802D7,
    0x14A195640116F336,    0x7C0828DD624EC390,    0xD74BBE77E6116AC7,    0x804456AF10F5FB53,
    0xEBE9EA2ADF4321C7,    0x03219A39EE587A30,    0x49787FEF17AF9924,    0xA1E9300CD8520548,
    0x5B45E522E4B1B4EF,    0xB49C3B3995091A36,    0xD4490AD526F14431,    0x12A8F216AF9418C2,
    0x001F837CC7350524,    0x1877B51E57A764D5,    0xA2853B80F17F58EE,    0x993E1DE72D36D310,
    0xB3598080CE64A656,    0x252F59CF0D9F04BB,    0xD23C8E176D113600,    0x1BDA0492E7E4586E,
    0x21E0BD5026C619BF,    0x3B097ADAF088F94E,    0x8D14DEDB30BE846E,    0xF95CFFA23AF5F6F4,
    0x3871700761B3F743,    0xCA672B91E9E4FA16,    0x64C8E531BFF53B55,    0x241260ED4AD1E87D,
    0x106C09B972D2E822,    0x7FBA195410E5CA30,    0x7884D9BC6CB569D8,    0x0647DFEDCD894A29,
    0x63573FF03E224774,    0x4FC8E9560F91B123,    0x1DB956E450275779,    0xB8D91274B9E9D4FB,
    0xA2EBEE47E2FBFCE1,    0xD9F1F30CCD97FB09,    0xEFED53D75FD64E6B,    0x2E6D02C36017F67F,
    0xA9AA4D20DB084E9B,    0xB64BE8D8B25396C1,    0x70CB6AF7C2D5BCF0,    0x98F076A4F7A2322E,
    0xBF84470805E69B5F,    0x94C3251F06F90CF3,    0x3E003E616A6591E9,    0xB925A6CD0421AFF3,
    0x61BDD1307C66E300,    0xBF8D5108E27E0D48,    0x240AB57A8B888B20,    0xFC87614BAF287E07,
    0xEF02CDD06FFDB432,    0xA1082C0466DF6C0A,    0x8215E577001332C8,    0xD39BB9C3A48DB6CF,
    0x2738259634305C14,    0x61CF4F94C97DF93D,    0x1B6BACA2AE4E125B,    0x758F450C88572E0B,
    0x959F587D507A8359,    0xB063E962E045F54D,    0x60E8ED72C0DFF5D1,    0x7B64978555326F9F,
    0xFD080D236DA814BA,    0x8C90FD9B083F4558,    0x106F72FE81E2C590,    0x7976033A39F7D952,
    0xA4EC0132764CA04B,    0x733EA705FAE4FA77,    0xB4D8F77BC3E56167,    0x9E21F4F903B33FD9,
    0x9D765E419FB69F6D,    0xD30C088BA61EA5EF,    0x5D94337FBFAF7F5B,    0x1A4E4822EB4D7A59,
    0x6FFE73E81B637FB3,    0xDDF957BC36D8B9CA,    0x64D0E29EEA8838B3,    0x08DD9BDFD96B9F63,
    0x087E79E5A57D1D13,    0xE328E230E3E2B3FB,    0x1C2559E30F0946BE,    0x720BF5F26F4D2EAA,
    0xB0774D261CC609DB,    0x443F64EC5A371195,    0x4112CF68649A260E,    0xD813F2FAB7F5C5CA,
    0x660D3257380841EE,    0x59AC2C7873F910A3,    0xE846963877671A17,    0x93B633ABFA3469F8,
    0xC0C0F5A60EF4CDCF,    0xCAF21ECD4377B28C,    0x57277707199B8175,    0x506C11B9D90E8B1D,
    0xD83CC2687A19255F,    0x4A29C6465A314CD1,    0xED2DF21216235097,    0xB5635C95FF7296E2,
    0x22AF003AB672E811,    0x52E762596BF68235,    0x9AEBA33AC6ECC6B0,    0x944F6DE09134DFB6,
    0x6C47BEC883A7DE39,    0x6AD047C430A12104,    0xA5B1CFDBA0AB4067,    0x7C45D833AFF07862,
    0x5092EF950A16DA0B,    0x9338E69C052B8E7B,    0x455A4B4CFE30E3F5,    0x6B02E63195AD0CF8,
    0x6B17B224BAD6BF27,    0xD1E0CCD25BB9C169,    0xDE0C89A556B9AE70,    0x50065E535A213CF6,
    0x9C1169FA2777B874,    0x78EDEFD694AF1EED,    0x6DC93D9526A50E68,    0xEE97F453F06791ED,
    0x32AB0EDB696703D3,    0x3A6853C7E70757A7,    0x31865CED6120F37D,    0x67FEF95D92607890,
    0x1F2B1D1F15F6DC9C,    0xB69E38A8965C6B65,    0xAA9119FF184CCCF4,    0xF43C732873F24C13,
    0xFB4A3D794A9A80D2,    0x3550C2321FD6109C,    0x371F77E76BB8417E,    0x6BFA9AAE5EC05779,
    0xCD04F3FF001A4778,    0xE3273522064480CA,    0x9F91508BFFCFC14A,    0x049A7F41061A9E60,
    0xFCB6BE43A9F2FE9B,    0x08DE8A1C7797DA9B,    0x8F9887E6078735A1,    0xB5B4071DBFC73A66,
    0x230E343DFBA08D33,    0x43ED7F5A0FAE657D,    0x3A88A0FBBCB05C63,    0x21874B8B4D2DBC4F,
    0x1BDEA12E35F6A8C9,    0x53C065C6C8E63528,    0xE34A1D250E7A8D6B,    0xD6B04D3B7651DD7E,
    0x5E90277E7CB39E2D,    0x2C046F22062DC67D,    0xB10BB459132D0A26,    0x3FA9DDFB67E2F199,
    0x0E09B88E1914F7AF,    0x10E8B35AF3EEAB37,    0x9EEDECA8E272B933,    0xD4C718BC4AE8AE5F,
    0x81536D601170FC20,    0x91B534F885818A06,    0xEC8177F83F900978,    0x190E714FADA5156E,
    0xB592BF39B0364963,    0x89C350C893AE7DC1,    0xAC042E70F8B383F2,    0xB49B52E587A1EE60,
    0xFB152FE3FF26DA89,    0x3E666E6F69AE2C15,    0x3B544EBE544C19F9,    0xE805A1E290CF2456,
    0x24B33C9D7ED25117,    0xE74733427B72F0C1,    0x0A804D18B7097475,    0x57E3306D881EDB4F,
    0x4AE7D6A36EB5DBCB,    0x2D8D5432157064C8,    0xD1E649DE1E7F268B,    0x8A328A1CEDFE552C,
    0x07A3AEC79624C7DA,    0x84547DDC3E203C94,    0x990A98FD5071D263,    0x1A4FF12616EEFC89,
    0xF6F7FD1431714200,    0x30C05B1BA332F41C,    0x8D2636B81555A786,    0x46C9FEB55D120902,
    0xCCEC0A73B49C9921,    0x4E9D2827355FC492,    0x19EBB029435DCB0F,    0x4659D2B743848A2C,
    0x963EF2C96B33BE31,    0x74F85198B05A2E7D,    0x5A0F544DD2B1FB18,    0x03727073C2E134B1,
    0xC7F6AA2DE59AEA61,    0x352787BAA0D7C22F,    0x9853EAB63B5E0B35,    0xABBDCDD7ED5C0860,
    0xCF05DAF5AC8D77B0,    0x49CAD48CEBF4A71E,    0x7A4C10EC2158C4A6,    0xD9E92AA246BF719E,
    0x13AE978D09FE5557,    0x730499AF921549FF,    0x4E4B705B92903BA4,    0xFF577222C14F0A3A,
    0x55B6344CF97AAFAE,    0xB862225B055B6960,    0xCAC09AFBDDD2CDB4,    0xDAF8E9829FE96B5F,
    0xB5FDFC5D3132C498,    0x310CB380DB6F7503,    0xE87FBB46217A360E,    0x2102AE466EBB1148,
    0xF8549E1A3AA5E00D,    0x07A69AFDCC42261A,    0xC4C118BFE78FEAAE,    0xF9F4892ED96BD438,
    0x1AF3DBE25D8F45DA,    0xF5B4B0B0D2DEEEB4,    0x962ACEEFA82E1C84,    0x046E3ECAAF453CE9,
    0xF05D129681949A4C,    0x964781CE734B3C84,    0x9C2ED44081CE5FBD,    0x522E23F3925E319E,
    0x177E00F9FC32F791,    0x2BC60A63A6F3B3F2,    0x222BBFAE61725606,    0x486289DDCC3D6780,
    0x7DC7785B8EFDFC80,    0x8AF38731C02BA980,    0x1FAB64EA29A2DDF7,    0xE4D9429322CD065A,
    0x9DA058C67844F20C,    0x24C0E332B70019B0,    0x233003B5A6CFE6AD,    0xD586BD01C5C217F6,
    0x5E5637885F29BC2B,    0x7EBA726D8C94094B,    0x0A56A5F0BFE39272,    0xD79476A84EE20D06,
    0x9E4C1269BAA4BF37,    0x17EFEE45B0DEE640,    0x1D95B0A5FCF90BC6,    0x93CBE0B699C2585D,
    0x65FA4F227A2B6D79,    0xD5F9E858292504D5,    0xC2B5A03F71471A6F,    0x59300222B4561E00,
    0xCE2F8642CA0712DC,    0x7CA9723FBB2E8988,    0x2785338347F2BA08,    0xC61BB3A141E50E8C,
    0x150F361DAB9DEC26,    0x9F6A419D382595F4,    0x64A53DC924FE7AC9,    0x142DE49FFF7A7C3D,
    0x0C335248857FA9E7,    0x0A9C32D5EAE45305,    0xE6C42178C4BBB92E,    0x71F1CE2490D20B07,
    0xF1BCC3D275AFE51A,    0xE728E8C83C334074,    0x96FBF83A12884624,    0x81A1549FD6573DA5,
    0x5FA7867CAF35E149,    0x56986E2EF3ED091B,    0x917F1DD5F8886C61,    0xD20D8C88C8FFE65F,
    0x31D71DCE64B2C310,    0xF165B587DF898190,    0xA57E6339DD2CF3A0,    0x1EF6E6DBB1961EC9,
    0x70CC73D90BC26E24,    0xE21A6B35DF0C3AD7,    0x003A93D8B2806962,    0x1C99DED33CB890A1,
    0xCF3145DE0ADD4289,    0xD0E4427A5514FB72,    0x77C621CC9FB3A483,    0x67A34DAC4356550B,
    0xF8D626AAAF278509,
];

// ---------------------------------------------------------------------------
// Polyglot Zobrist hash
// ---------------------------------------------------------------------------

/// Computes the Polyglot Zobrist hash of a position.
///
/// Reference algorithm (see module doc): XOR of the constants
/// corresponding to each piece present, to the castling rights still
/// valid, to the en passant square *actually* playable, and to the side to move.
///
/// Clippy (04/07/2026): `#[allow(cast_possible_truncation)]` — the index into
/// `RANDOM_ARRAY` (`64 * piece_index + sq.index()`) always stays within
/// `0..768` (`piece_index` ∈ `0..=11`, `sq.index()` ∈ `0..=63`), well
/// within the `usize` range even on a 32-bit target; no truncation is possible
/// in practice.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn polyglot_hash(pos: &Position) -> u64 {
    let mut hash = 0u64;

    for (sq, piece) in pos.board.pieces() {
        let type_offset: u64 = match piece.kind {
            PieceKind::Pawn => 0,
            PieceKind::Knight => 1,
            PieceKind::Bishop => 2,
            PieceKind::Rook => 3,
            PieceKind::Queen => 4,
            PieceKind::King => 5,
        };
        let color_pivot: u64 = match piece.color {
            Color::Black => 0,
            Color::White => 1,
        };
        let piece_index = type_offset * 2 + color_pivot;
        hash ^= RANDOM_ARRAY[(64 * piece_index) as usize + sq.index() as usize];
    }

    if pos.castling.white_kingside {
        hash ^= RANDOM_ARRAY[768];
    }
    if pos.castling.white_queenside {
        hash ^= RANDOM_ARRAY[769];
    }
    if pos.castling.black_kingside {
        hash ^= RANDOM_ARRAY[770];
    }
    if pos.castling.black_queenside {
        hash ^= RANDOM_ARRAY[771];
    }

    if let Some(ep) = pos.en_passant {
        if en_passant_capturable(pos, ep) {
            hash ^= RANDOM_ARRAY[772 + ep.file() as usize];
        }
    }

    if pos.side_to_move == Color::White {
        hash ^= RANDOM_ARRAY[780];
    }

    hash
}

/// Does the en passant square `ep` correspond to an *actually*
/// possible capture (adjacent opponent pawn, side to move)? The Polyglot format only
/// hashes en passant in this exact case — recording the square in the
/// FEN is not enough.
fn en_passant_capturable(pos: &Position, ep: Square) -> bool {
    let capturing_color = pos.side_to_move;
    let capturing_rank: i16 = match capturing_color {
        Color::White => i16::from(ep.rank()) - 1,
        Color::Black => i16::from(ep.rank()) + 1,
    };
    if !(0..=7).contains(&capturing_rank) {
        return false;
    }
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let capturing_rank = capturing_rank as u8;

    let mut adjacent_files = Vec::with_capacity(2);
    if ep.file() > 0 {
        adjacent_files.push(ep.file() - 1);
    }
    if ep.file() < 7 {
        adjacent_files.push(ep.file() + 1);
    }

    let expected = Piece::new(capturing_color, PieceKind::Pawn);
    adjacent_files
        .into_iter()
        .any(|file| pos.piece_at(Square::new(file, capturing_rank)) == Some(expected))
}

// ---------------------------------------------------------------------------
// Decoding a raw Polyglot move
// ---------------------------------------------------------------------------

/// Decodes a raw Polyglot move (`u16`) into UCI notation.
///
/// Accounts for the format's castling quirk: the encoded move
/// is "the king captures its own rook" (destination square = starting
/// square of the rook involved), and must be converted to the king's
/// standard visual destination square (g1/c1/g8/c8) to be usable
/// as a normal UCI move.
fn decode_move(raw: u16, pos: &Position) -> String {
    let to_index = (raw & 0x3f) as u8;
    let from_index = ((raw >> 6) & 0x3f) as u8;
    let promotion_part = (raw >> 12) & 0x7;

    let from = Square::from_index(from_index);
    let mut to = Square::from_index(to_index);

    if let Some(moving) = pos.piece_at(from) {
        if moving.kind == PieceKind::King {
            if let Some(target) = pos.piece_at(to) {
                if target.kind == PieceKind::Rook && target.color == moving.color {
                    let rank = from.rank();
                    to = if to.file() > from.file() {
                        Square::new(6, rank) // Kingside castling: g1 / g8.
                    } else {
                        Square::new(2, rank) // Queenside castling: c1 / c8.
                    };
                }
            }
        }
    }

    let promotion = match promotion_part {
        1 => Some(PieceKind::Knight),
        2 => Some(PieceKind::Bishop),
        3 => Some(PieceKind::Rook),
        4 => Some(PieceKind::Queen),
        _ => None,
    };
    let promo_char = promotion.map_or(String::new(), |p| p.fen_char().to_string());

    format!("{from}{to}{promo_char}")
}

// ---------------------------------------------------------------------------
// Polyglot book
// ---------------------------------------------------------------------------

/// An entry of a Polyglot book (corresponds to a 16-byte record).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolyglotEntry {
    pub key: u64,
    pub raw_move: u16,
    pub weight: u16,
}

/// Error while loading a Polyglot book file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolyglotError {
    /// The file size is not a (nonzero) multiple of 16 bytes.
    InvalidSize { size: usize },
    /// The file is not sorted by increasing hash key — a condition
    /// required for binary search, and an almost certain sign that it is
    /// not a genuine Polyglot file.
    NotSorted,
    /// Error reading the file from disk.
    Io(String),
}

impl std::fmt::Display for PolyglotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSize { size } => write!(
                f,
                "Fichier invalide ({size} octets) : un livre Polyglot valide doit \
                 avoir une taille multiple de 16 octets."
            ),
            Self::NotSorted => write!(
                f,
                "Fichier non trié par clé de hachage croissante : ce n'est \
                 probablement pas un livre Polyglot valide."
            ),
            Self::Io(msg) => write!(f, "Erreur de lecture du fichier : {msg}"),
        }
    }
}

impl std::error::Error for PolyglotError {}

/// A Polyglot opening book loaded into memory.
#[derive(Debug, Clone, PartialEq)]
pub struct PolyglotBook {
    entries: Vec<PolyglotEntry>,
}

impl PolyglotBook {
    /// Loads and validates a book from a `.bin` file.
    ///
    /// # Errors
    /// See [`PolyglotError`]: invalid size, unsorted file, or disk
    /// read error.
    pub fn open(path: &std::path::Path) -> Result<Self, PolyglotError> {
        let bytes = std::fs::read(path).map_err(|e| PolyglotError::Io(e.to_string()))?;
        Self::from_bytes(&bytes)
    }

    /// Builds and validates a book from a buffer already in memory.
    ///
    /// # Errors
    /// See [`Self::open`].
    ///
    /// # Panics
    /// Does not panic in practice: the internal `.unwrap()`s convert
    /// slices coming from `bytes.chunks_exact(16)`, which guarantees
    /// chunks of exactly 16 bytes — the sub-slices `[0..8]`/`[8..10]`/
    /// `[10..12]` therefore always have the size expected by `try_into()`.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PolyglotError> {
        if bytes.is_empty() || !bytes.len().is_multiple_of(16) {
            return Err(PolyglotError::InvalidSize { size: bytes.len() });
        }

        let mut entries = Vec::with_capacity(bytes.len() / 16);
        for chunk in bytes.chunks_exact(16) {
            let key = u64::from_be_bytes(chunk[0..8].try_into().unwrap());
            let raw_move = u16::from_be_bytes(chunk[8..10].try_into().unwrap());
            let weight = u16::from_be_bytes(chunk[10..12].try_into().unwrap());
            // The last 4 bytes (learning value) are not used here.
            entries.push(PolyglotEntry { key, raw_move, weight });
        }

        if !entries.windows(2).all(|w| w[0].key <= w[1].key) {
            return Err(PolyglotError::NotSorted);
        }

        Ok(Self { entries })
    }

    /// Number of entries in the book.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Is the book empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries matching an exact hash key (binary search).
    fn entries_for_key(&self, key: u64) -> &[PolyglotEntry] {
        let start = self.entries.partition_point(|e| e.key < key);
        let end = start + self.entries[start..].partition_point(|e| e.key == key);
        &self.entries[start..end]
    }

    /// Chooses a move for the given position, drawn at random with a
    /// probability proportional to each candidate's Polyglot weight
    /// (game variety, as any software using Polyglot does).
    ///
    /// Returns `None` if the position is not in the book.
    ///
    /// The randomness source follows the convention already in place in the
    /// project (see `GameConfig::resolve_human_is_white`): system clock,
    /// with no dependency on the `rand` crate.
    #[must_use]
    pub fn pick_uci_move(&self, pos: &Position) -> Option<String> {
        let key = polyglot_hash(pos);
        let candidates = self.entries_for_key(key);
        if candidates.is_empty() {
            return None;
        }

        let total_weight: u32 = candidates.iter().map(|e| u32::from(e.weight)).sum();
        let chosen = if total_weight == 0 {
            // No weight given: equiprobable among the candidates.
            let idx = (random_u32() as usize) % candidates.len();
            candidates[idx]
        } else {
            let mut target = random_u32() % total_weight;
            let mut selected = candidates[candidates.len() - 1];
            for entry in candidates {
                let w = u32::from(entry.weight);
                if target < w {
                    selected = *entry;
                    break;
                }
                target -= w;
            }
            selected
        };

        Some(decode_move(chosen.raw_move, pos))
    }
}

/// Simple pseudo-random number derived from the system clock.
fn random_u32() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Critical check: the hash of the starting position must
    /// match exactly the constant published by the Polyglot format.
    /// If this test fails, either the `RANDOM_ARRAY` table or the algorithm is wrong.
    #[test]
    fn test_starting_position_hash() {
        assert_eq!(polyglot_hash(&Position::starting()), 0x463b_9618_1691_fc9c);
    }

    /// Official test vectors from `python-chess` (`test_polyglot`), which
    /// cover: pieces, partial/total loss of castling rights, and
    /// en passant that is actually/not actually playable.
    #[test]
    fn test_known_vectors_from_python_chess() {
        let cases: &[(&str, u64)] = &[
            (
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
                0x463b_9618_1691_fc9c,
            ),
            (
                "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
                0x823c_9b50_fd11_4196,
            ),
            (
                "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
                0x0756_b944_61c5_0fb0,
            ),
            (
                "rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR b KQkq - 0 2",
                0x662f_afb9_65db_29d4,
            ),
            // En passant actually playable (white pawn on e5 adjacent
            // to the black pawn that just played f7-f5): the f-file bit must
            // be hashed.
            (
                "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3",
                0x22a4_8b5a_8e47_ff78,
            ),
            // White king moved: white castling rights lost, black intact.
            (
                "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPPKPPP/RNBQ1BNR b kq - 1 3",
                0x652a_607c_a3f2_42c1,
            ),
            // Both kings have moved: no more castling rights at all.
            (
                "rnbq1bnr/ppp1pkpp/8/3pPp2/8/8/PPPPKPPP/RNBQ1BNR w - - 2 4",
                0x00fd_d303_c946_bdd9,
            ),
            // Another case of en passant actually playable (c-file).
            (
                "rnbqkbnr/p1pppppp/8/8/PpP4P/8/1P1PPPP1/RNBQKBNR b KQkq c3 0 3",
                0x3c81_23ea_7b06_7637,
            ),
            // After the en passant capture + Ra3: rook a1 moved, white
            // queenside castling right lost.
            (
                "rnbqkbnr/p1pppppp/8/8/P6P/R1p5/1P1PPPP1/1NBQKBNR b Kkq - 1 4",
                0x5c3f_9b82_9b27_9560,
            ),
        ];

        for (fen, expected) in cases {
            let pos = Position::from_fen(fen).unwrap();
            assert_eq!(polyglot_hash(&pos), *expected, "hachage incorrect pour {fen}");
        }
    }

    #[test]
    fn test_en_passant_not_capturable_returns_false() {
        // Starting position manually modified: ep square declared but
        // no adjacent opponent pawn can actually capture.
        let mut pos = Position::starting();
        pos.en_passant = Some(Square::from_algebraic("e3").unwrap());
        pos.side_to_move = Color::Black;
        assert!(!en_passant_capturable(&pos, Square::from_algebraic("e3").unwrap()));
    }

    #[test]
    fn test_en_passant_capturable_true_case() {
        use crate::types::Board;
        let mut board = Board::empty();
        board.set_piece(
            Square::from_algebraic("d4").unwrap(),
            Some(Piece::new(Color::Black, PieceKind::Pawn)),
        );
        let pos = Position {
            board,
            side_to_move: Color::Black,
            castling: crate::types::CastlingRights::none(),
            en_passant: Some(Square::from_algebraic("e3").unwrap()),
            halfmove_clock: 0,
            fullmove_number: 1,
        };
        assert!(en_passant_capturable(&pos, Square::from_algebraic("e3").unwrap()));
    }

    #[test]
    fn test_decode_normal_move() {
        // e2 (index 12) -> e4 (index 28), no promotion.
        let raw: u16 = (12 << 6) | 0x001c;
        let uci = decode_move(raw, &Position::starting());
        assert_eq!(uci, "e2e4");
    }

    #[test]
    fn test_decode_promotion_move() {
        // a7 (index 48) -> a8 (index 56), queen promotion (4).
        let raw: u16 = (4 << 12) | (48 << 6) | 0x0038;
        let mut pos = Position::starting();
        pos.set_piece(
            Square::from_algebraic("a7").unwrap(),
            Some(Piece::new(Color::White, PieceKind::Pawn)),
        );
        pos.set_piece(Square::from_algebraic("a8").unwrap(), None);
        let uci = decode_move(raw, &pos);
        assert_eq!(uci, "a7a8q");
    }

    #[test]
    fn test_decode_white_kingside_castle() {
        // Polyglot convention: the king "captures" its own rook (e1 -> h1).
        let raw: u16 = (4 << 6) | 7;
        let uci = decode_move(raw, &Position::starting());
        assert_eq!(uci, "e1g1");
    }

    #[test]
    fn test_decode_white_queenside_castle() {
        // e1 -> a1.
        let raw: u16 = 4 << 6;
        let uci = decode_move(raw, &Position::starting());
        assert_eq!(uci, "e1c1");
    }

    #[test]
    fn test_decode_black_kingside_castle() {
        // e8 (index 60) -> h8 (index 63).
        let raw: u16 = (60 << 6) | 63;
        let uci = decode_move(raw, &Position::starting());
        assert_eq!(uci, "e8g8");
    }

    #[test]
    fn test_from_bytes_rejects_invalid_size() {
        let bytes = vec![0u8; 17];
        assert!(matches!(
            PolyglotBook::from_bytes(&bytes),
            Err(PolyglotError::InvalidSize { size: 17 })
        ));
    }

    #[test]
    fn test_from_bytes_rejects_empty() {
        assert!(matches!(
            PolyglotBook::from_bytes(&[]),
            Err(PolyglotError::InvalidSize { size: 0 })
        ));
    }

    #[test]
    fn test_from_bytes_rejects_unsorted() {
        let mut bytes = Vec::new();
        // Two entries deliberately in the wrong order (decreasing key).
        bytes.extend_from_slice(&2u64.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&1u64.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(PolyglotBook::from_bytes(&bytes), Err(PolyglotError::NotSorted));
    }

    #[test]
    fn test_from_bytes_accepts_valid_sorted_book() {
        let key = polyglot_hash(&Position::starting());
        let raw_move: u16 = (12 << 6) | 0x001c; // e2e4
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&key.to_be_bytes());
        bytes.extend_from_slice(&raw_move.to_be_bytes());
        bytes.extend_from_slice(&10u16.to_be_bytes()); // weight
        bytes.extend_from_slice(&0u32.to_be_bytes());

        let book = PolyglotBook::from_bytes(&bytes).unwrap();
        assert_eq!(book.len(), 1);
        assert!(!book.is_empty());
        assert_eq!(book.pick_uci_move(&Position::starting()), Some("e2e4".to_string()));
    }

    #[test]
    fn test_pick_uci_move_returns_none_when_position_absent() {
        // Valid book but containing only an entry for a different position.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u64.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        let book = PolyglotBook::from_bytes(&bytes).unwrap();
        assert_eq!(book.pick_uci_move(&Position::starting()), None);
    }

    #[test]
    fn test_random_array_starting_constant_present() {
        // Indirect sanity check: the table has the right size (enforced by
        // the type) and is not filled with zeros (construction error).
        assert_eq!(RANDOM_ARRAY.len(), 781);
        assert!(RANDOM_ARRAY.iter().any(|&v| v != 0));
    }
}
