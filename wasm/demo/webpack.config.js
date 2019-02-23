const HtmlWebpackPlugin = require('html-webpack-plugin');
const MiniCssExtractPlugin = require('mini-css-extract-plugin');
const WasmPackPlugin = require('@wasm-tool/wasm-pack-plugin');
const path = require('path');
const fs = require('fs');

module.exports = {
    entry: './src/index.js',
    output: {
        path: path.join(__dirname, 'dist'),
        filename: 'index.js'
    },
    mode: 'development',
    module: {
        rules: [
            {
                test: /\.css$/,
                use: [MiniCssExtractPlugin.loader, 'css-loader']
            }
        ]
    },
    plugins: [
        new HtmlWebpackPlugin({
            filename: 'index.html',
            template: 'src/index.ejs',
            templateParameters: {
                snippets: fs
                    .readdirSync(path.join(__dirname, 'src/snippets'))
                    .map(filename =>
                        path.basename(filename, path.extname(filename))
                    ),
                defaultSnippetName: 'fibonacci',
                defaultSnippet: fs.readFileSync(
                    path.join(__dirname, 'src/snippets/fibonacci.py')
                )
            }
        }),
        new MiniCssExtractPlugin({
            filename: 'styles.css'
        }),
        new WasmPackPlugin({
            crateDirectory: path.join(__dirname, '../lib')
        })
    ]
};
