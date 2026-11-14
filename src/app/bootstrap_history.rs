use super::BootstrapPackageManifest;

pub(super) const LEGACY_SINGLE_FILE_BOOTSTRAPS: &[(&str, &str)] = &[
    (
        "1.0.0",
        "08440a4a3e10489eae2484d0a5bf8f4b0451d22c43edaa90c75fcd0b66fd4a74",
    ),
    (
        "1.0.1",
        "2e18b702cbe61660aafff0b4c01538b51561cd0ce979cc4cce914f539b8c755e",
    ),
    (
        "1.0.2",
        "4a6c9de673948dbccfe719973395b7712fe09706ae22b2119e4a8ed8c5446a73",
    ),
    (
        "1.0.3",
        "1c0ffe7e1ef65d88e5c42a8978d92de81353c69072d8e610dce0c33d3938290c",
    ),
    (
        "1.0.4",
        "629d2d959c5b4277398e48f7b10c29c6c2adb0c0415723938ad7bbe4f8fec1c8",
    ),
    (
        "1.1.0",
        "d463d28295055fde8012e5c2424a754bf93e8c84c9fd6b48b7194bbaff0b27c2",
    ),
    (
        "1.2.0",
        "a12e3bbf45e4a3d51b8075dd2b56a410f113fb27cd803676fb614bd88f278f9c",
    ),
    (
        "1.3.0",
        "0fe2e7b8aed1f6db1c57a3d5b6efc247b795e1af01d5422b9168e707bc8c7b47",
    ),
    (
        "1.4.0",
        "2161ba4834fb350afee75fd6ab90ecff49741c971bb2d670b8d3c9f6588f9b70",
    ),
    (
        "1.5.0",
        "0afb58572bf602024f78e0f8c7312cc3485abb184746898945d1f7501a5425b5",
    ),
    (
        "1.5.1",
        "abb33e147e1b092ff70a2b02c5a8f89fc70d0c73eed2d8ec5ea88bba1ae58221",
    ),
    (
        "1.6.0",
        "c5f05692527bb2b8c45012ba6a464268b6a91a683a429a11dbe67022bbcc2aa5",
    ),
    (
        "1.7.0",
        "8b503801efa6a5dd13c8767647654bb9fcd4dcfaf10cdbbb61429dfe29995129",
    ),
    (
        "1.7.1",
        "f742e5d1af14309e3fa7e95378a207fafed3d09e477b1df9c9e76d65e4eec304",
    ),
    (
        "1.8.0",
        "820b760c5f6cc8c2ad6695e211044882077af138014579236b8824039fd1f394",
    ),
    (
        "1.8.1",
        "996891144bca51a654fc51294cd6d63c41df4afb5796af7846480b168f69edc6",
    ),
    (
        "1.8.2",
        "d79be31fe878267d00f21cf8e198443ebf8b95eb6631fc2395f132f12289e3e5",
    ),
    (
        "1.8.3",
        "3eba54753cfe8cdf987a8a4fe1ab1337317aef9b72e55b9be49b3158117470b1",
    ),
    (
        "1.8.4",
        "2758bed89792128c47d01f613a4df277dd32c9641cb036e7ed1f03c0c94e8381",
    ),
    (
        "1.8.5",
        "9f25609e002ad63e750214c54d7138f6e877849e46626aef441e00eaca8738d1",
    ),
    (
        "1.8.6",
        "4412ddc1fa4006492c28f8813462504a68cc71f4025398c3d60d5c225b2b904b",
    ),
    (
        "1.8.7",
        "68007e9c80a942cf4afa581f90639bbd260b69a36cb72aebdbca63e92d787eed",
    ),
    (
        "1.8.8",
        "15dc6c0bd25dc7a6e97336124960b7d548ecea9a3a58b34d55a4d57dbc4d5aeb",
    ),
    (
        "1.8.9",
        "1087110d8f8f9bb2c1839b1eda11b4417ca7b57af4449113a6fb5b1394e3309d",
    ),
    (
        "1.8.10",
        "dbc604dbfc45db5bf766609b28513eacb1ba87a96fc0cba7bb11337561bddd01",
    ),
    (
        "1.8.11",
        "501c0ce97d677b1a3c68d25dd1ece99590895dd51e23e671ea96312925e98c59",
    ),
    (
        "1.8.12",
        "69bf20da3494b69b634aee5e8d7dead8d8e30222f9e380461640af02bc83331e",
    ),
    (
        "1.8.13",
        "3063f8913c22846640389f5a633697aea253ab0805e974bd19fa0f06f2b01682",
    ),
    (
        "1.8.14",
        "ebf48e298ece30cd850146362b3fa73950b4fa26e35f3ce774a5700ae8b3e3cb",
    ),
    (
        "1.8.15",
        "dd08e0f4140989cb2a1178f60e152ee7913377d743f4029b9699122589193d47",
    ),
    (
        "1.8.16",
        "4d46b9699db8490821c1f91be1444dca689cd4b9578a2b732b633de441f8ff59",
    ),
    (
        "1.8.17",
        "5c7cc974e0d83d82993ba1ca9227c0d542288ca3d7ef4e04165794cb41faec33",
    ),
    (
        "1.8.18",
        "130cb488305b53dc3123633da50df6d3b843b24663861f7cc6b63964621b8718",
    ),
    (
        "1.8.19",
        "78aa0dbdf53c3f9f08fb5bf9805e33b2d5b8ffe4c63bac05d88391f608fab16c",
    ),
    (
        "1.8.20",
        "fc1318f0a8587ce193ab1f54462374508026cf8c50a382b613dc18b48f9d09f2",
    ),
];

pub(super) const LEGACY_COMPLETE_BOOTSTRAP_PACKAGES: &[BootstrapPackageManifest<'static>] = &[
    BootstrapPackageManifest {
        version: "1.8.53",
        file_digests: &[
            (
                "SKILL.md",
                "18fdc26bb0e711b44416dfd47ddc51fde09fbd4c304ae7d1ade7398b21b65cda",
            ),
            (
                "references/routing.md",
                "4d1d2bf283c4d9d5ea0471f38eaa9479ac5d84be2b764d3247e20b0068e267e4",
            ),
            (
                "references/governance.md",
                "78965720f8e776e2486770142a04e204bc88fd21c6af318177c6dbddc1678d51",
            ),
            (
                "references/mutation.md",
                "40a8529fcdc847eb52b0e10d9fd19a792b5379392199109ab2dab0a6ed12fd13",
            ),
        ],
    },
    BootstrapPackageManifest {
        version: "1.8.52",
        file_digests: &[
            (
                "SKILL.md",
                "12185a2872878bcd4d1e871287a31cc78a1c6eb24bef19c8f735265bd34693ff",
            ),
            (
                "references/routing.md",
                "5783fc2177e513273dba0bc3bc80b061e5bc1a628117e6e7fb42644b774885e7",
            ),
            (
                "references/governance.md",
                "a8fa9e850308cd7529c86a8882d73cb29f09951d9e24baa2cef4cad11dbca05b",
            ),
            (
                "references/mutation.md",
                "369aa2d07c59cfda5e82bc7f599528eb41cdd01bf78a0042a9baa6461563475f",
            ),
        ],
    },
    BootstrapPackageManifest {
        version: "1.8.51",
        file_digests: &[
            (
                "SKILL.md",
                "4555b296030a0680d820ff3b607c6ddb3874c18677c5efafebf333c3086e3d36",
            ),
            (
                "references/routing.md",
                "83ce87d14166347630ad8951597151092bc0b3be7cc7bd9f0b1d22795689640b",
            ),
            (
                "references/governance.md",
                "9bb2b3366d0bdc7decafd4370123a8d94c04d7aea4b39595d8b52e7e6ac79f52",
            ),
            (
                "references/mutation.md",
                "0f2b4089903a54b6087b159ddd27f7fca44e6be471d54b98f1bb4217a6223101",
            ),
        ],
    },
    BootstrapPackageManifest {
        version: "1.8.50",
        file_digests: &[
            (
                "SKILL.md",
                "2cb9d6aa446c8cba2a8cd57e36ed256edd6655110006e64df173bfb94bfddcd1",
            ),
            (
                "references/routing.md",
                "58bd0140cf31ebf89b7ffbc9d0d6f5013096897e666c436b8bf703ce45abd3cf",
            ),
            (
                "references/governance.md",
                "9bb2b3366d0bdc7decafd4370123a8d94c04d7aea4b39595d8b52e7e6ac79f52",
            ),
            (
                "references/mutation.md",
                "0f2b4089903a54b6087b159ddd27f7fca44e6be471d54b98f1bb4217a6223101",
            ),
        ],
    },
    BootstrapPackageManifest {
        version: "1.8.49",
        file_digests: &[
            (
                "SKILL.md",
                "03dc9acfc37a979d1517640959fdf1572f55e673cd1f4884adc86a5f7223810a",
            ),
            (
                "references/routing.md",
                "58bd0140cf31ebf89b7ffbc9d0d6f5013096897e666c436b8bf703ce45abd3cf",
            ),
            (
                "references/governance.md",
                "ad540ca44ea39e386e1a8e4504dcb574ca6bf1baa4ff35f441ea4eac382952f9",
            ),
            (
                "references/mutation.md",
                "b6614b2e0c52f1b84df1562462810b0fdfe03fabac0da75f8bde26693bb43589",
            ),
        ],
    },
    BootstrapPackageManifest {
        version: "1.8.23",
        file_digests: &[
            (
                "SKILL.md",
                "0b5ff8d4ab352ef5b78908a6619352fce668ff5cbdcbc4258c450ceb5a34637f",
            ),
            (
                "references/routing.md",
                "be2ccff81b8b85c298abaa7f59a71c2f44dc6f1c990804cdf0f901d715c2f782",
            ),
            (
                "references/governance.md",
                "3285223b046721c6719fa8c6fa3628f2d33488efb4b23bf334a9763f442e95b6",
            ),
            (
                "references/mutation.md",
                "6490e355033f9fe6de5d027902241a514449a59fbf87831f7971ab1b98687927",
            ),
        ],
    },
    BootstrapPackageManifest {
        version: "1.8.29",
        file_digests: &[
            (
                "SKILL.md",
                "c62bc41e54000a6191c781b45d082440cb5930bc26a1511dafc000cb1c1af5e3",
            ),
            (
                "references/routing.md",
                "9899e88dac1738584b6a8c615077b84f230a73292295a23e5614895628df0f22",
            ),
            (
                "references/governance.md",
                "3ba164010e51559708cb863abb9dfed450c065680256ddc348ec4677c7a01de8",
            ),
            (
                "references/mutation.md",
                "b6614b2e0c52f1b84df1562462810b0fdfe03fabac0da75f8bde26693bb43589",
            ),
        ],
    },
    BootstrapPackageManifest {
        version: "1.8.46",
        file_digests: &[
            (
                "SKILL.md",
                "8e6a7eacea3aea3fa7c11f6f1d73af06f5ca8b98cd287ab82004acd7b0fa938b",
            ),
            (
                "references/routing.md",
                "58bd0140cf31ebf89b7ffbc9d0d6f5013096897e666c436b8bf703ce45abd3cf",
            ),
            (
                "references/governance.md",
                "0aff10c04ef8aaa6fc19b119aa5e7b723fe7ae01f1bf72a569d0b02b7cdec5ad",
            ),
            (
                "references/mutation.md",
                "b6614b2e0c52f1b84df1562462810b0fdfe03fabac0da75f8bde26693bb43589",
            ),
        ],
    },
    BootstrapPackageManifest {
        version: "1.8.47",
        file_digests: &[
            (
                "SKILL.md",
                "01f346450b239524658f1082e701144c396119ac0ef0278739928c5d797253a0",
            ),
            (
                "references/routing.md",
                "58bd0140cf31ebf89b7ffbc9d0d6f5013096897e666c436b8bf703ce45abd3cf",
            ),
            (
                "references/governance.md",
                "0aff10c04ef8aaa6fc19b119aa5e7b723fe7ae01f1bf72a569d0b02b7cdec5ad",
            ),
            (
                "references/mutation.md",
                "b6614b2e0c52f1b84df1562462810b0fdfe03fabac0da75f8bde26693bb43589",
            ),
        ],
    },
    BootstrapPackageManifest {
        version: "1.8.48",
        file_digests: &[
            (
                "SKILL.md",
                "9b95e854f583f646b5aac0d2d839d249cd78651d9fb47a386f9c4d0c513a9e77",
            ),
            (
                "references/routing.md",
                "58bd0140cf31ebf89b7ffbc9d0d6f5013096897e666c436b8bf703ce45abd3cf",
            ),
            (
                "references/governance.md",
                "b121cfdbc3050efedbf2a603b2c7b61632285ea61a686204fcc855aeb115169e",
            ),
            (
                "references/mutation.md",
                "b6614b2e0c52f1b84df1562462810b0fdfe03fabac0da75f8bde26693bb43589",
            ),
        ],
    },
];
